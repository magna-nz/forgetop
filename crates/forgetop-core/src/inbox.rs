//! The notification inbox as both frontends draw it: one row per item (a *thread*), led by
//! the reason that asks most of you, labelled in plain words.
//!
//! A provider's feed is one entry per event — Linear sends a notification for every comment,
//! status move and reaction on an issue — so the same item arrives several times. Grouping is a
//! view over the rows already fetched (like work-item `parent` grouping): nothing here fetches,
//! and the cached rows keep their shape.

use chrono::{DateTime, Utc};

use crate::domain::{Notification, NotificationItemType, NotificationKind, NotificationTier};

/// One inbox row: every notification about the same item on the same connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxThread {
    /// Stable identity of the item the thread is about — see [`thread_key`].
    pub key: String,
    /// Index (into the slice given to [`threads`]) of the notification that labels the row.
    pub lead: usize,
    /// Every member's index, newest first, `lead` included — what "done" marks read.
    pub members: Vec<usize>,
    pub tier: NotificationTier,
    /// Any visible member still unread.
    pub unread: bool,
    pub latest: Option<DateTime<Utc>>,
}

impl InboxThread {
    /// How many more events the row stands for besides its lead (the `+N` on the row),
    /// not counting hidden ones such as reactions.
    pub fn more(&self, rows: &[(&str, &Notification)]) -> usize {
        self.members
            .iter()
            .filter(|&&i| i != self.lead && rows[i].1.kind.tier() != NotificationTier::Hidden)
            .count()
    }
}

/// Identity of the item a notification is about. Two notifications share a thread when they
/// name the same item on the same connection; one with no item id is a thread of its own.
pub fn thread_key(conn: &str, n: &Notification) -> String {
    match &n.item_id {
        Some(id) => format!(
            "{conn}\u{1f}{:?}\u{1f}{}\u{1f}{id}",
            n.item_type,
            n.repository.as_deref().unwrap_or_default()
        ),
        None => format!("{conn}\u{1f}notification\u{1f}{}", n.id),
    }
}

/// Identity of a notification for a local dismissal ("done"). It carries `updated_at` so a
/// thread with new activity after it was dismissed comes back.
pub fn dismiss_key(conn: &str, n: &Notification) -> String {
    let at = n.updated_at.map(|t| t.timestamp()).unwrap_or_default();
    format!("{conn}:{}:{at}", n.id)
}

/// Folds `rows` (each a connection id and its notification) into threads, newest first.
///
/// A thread's lead is its unread member that asks most of you (by [`NotificationKind::rank`]),
/// newest on a tie; once everything is read, the member that asked most. Its tier is For you
/// when any member is. A thread made only of hidden notifications (reactions) is left out.
pub fn threads(rows: &[(&str, &Notification)]) -> Vec<InboxThread> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
    for (i, (conn, n)) in rows.iter().enumerate() {
        let key = thread_key(conn, n);
        groups
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key);
                Vec::new()
            })
            .push(i);
    }

    let mut out: Vec<InboxThread> = order
        .into_iter()
        .filter_map(|key| {
            let mut members = groups.remove(&key)?;
            members.sort_by_key(|&i| std::cmp::Reverse(rows[i].1.updated_at));
            let visible: Vec<usize> =
                members.iter().copied().filter(|&i| rows[i].1.kind.tier() != NotificationTier::Hidden).collect();
            // While anything is unread, the unread member that asks most of you leads (newest on
            // a tie: `min_by_key` keeps the first); once all of it is read, the one that asked most.
            let any_unread = visible.iter().any(|&i| rows[i].1.unread);
            let lead = *visible
                .iter()
                .filter(|&&i| !any_unread || rows[i].1.unread)
                .min_by_key(|&&i| rows[i].1.kind.rank())?;
            // A thread is for you if anything in it was, read or not — so opening one never
            // moves it out of the inbox you opened it from, nor reading one into it.
            let tier = if visible.iter().any(|&i| rows[i].1.kind.tier() == NotificationTier::ForYou) {
                NotificationTier::ForYou
            } else {
                NotificationTier::Following
            };
            Some(InboxThread {
                key,
                lead,
                tier,
                unread: any_unread,
                latest: members.iter().filter_map(|&i| rows[i].1.updated_at).max(),
                members,
            })
        })
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.latest));
    out
}

/// What the inbox shows: the threads waiting on you ([`NotificationTier::ForYou`]), newest
/// first. A thread you only follow — comments, status moves, new issues where you watch — isn't
/// listed; the forge's own inbox still has it. Both frontends list exactly these.
pub fn inbox(rows: &[(&str, &Notification)]) -> Vec<InboxThread> {
    threads(rows).into_iter().filter(|t| t.tier == NotificationTier::ForYou).collect()
}

/// The first word of a display name — "Priya" for "Priya Nair". Handles stay whole.
fn first_name(actor: &str) -> &str {
    actor.split_whitespace().next().unwrap_or(actor)
}

/// What a notification is about, as a noun for the why line.
fn item_noun(t: NotificationItemType) -> &'static str {
    match t {
        NotificationItemType::PullRequest => "pull request",
        NotificationItemType::WorkItem => "issue",
        NotificationItemType::Pipeline => "pipeline",
        NotificationItemType::Other => "thread",
    }
}

/// A provider reason we file under [`NotificationKind::Other`] that still has a plain phrase.
fn known_other(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "author" => "on yours",
        "manual" | "subscribed" | "issueSubscribed" => "following",
        "issuePriorityUrgent" => "marked urgent",
        "issueBlocking" => "blocked",
        "issueUnblocked" => "unblocked",
        "issueUnassignedFromYou" => "unassigned",
        "issueReopened" => "reopened",
        "security_alert" => "security alert",
        "approval_requested" => "approval requested",
        "invitation" => "invitation",
        _ => return None,
    })
}

/// A raw provider reason in words: `issueTitleChanged` → "title changed",
/// `member_access_requested` → "member access requested".
fn humanize(reason: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in reason.chars() {
        if c == '_' || c == '-' || c == ' ' {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        } else if c.is_uppercase() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            cur.extend(c.to_lowercase());
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    // Linear prefixes every type with the noun it is about; the row already says that.
    if words.len() > 1 && words[0] == "issue" {
        words.remove(0);
    }
    words.join(" ")
}

/// The row's reason, in a few plain words: "Priya mentioned you", "→ In Review",
/// "new in Platform", "marked urgent". Never a bare category like "activity".
pub fn phrase(n: &Notification) -> String {
    let who = n.actor.as_deref().map(first_name);
    match n.kind {
        NotificationKind::Mention => match who {
            Some(a) => format!("{a} mentioned you"),
            None => "mentioned you".into(),
        },
        NotificationKind::ReviewRequested => "review requested".into(),
        NotificationKind::Assigned => "assigned to you".into(),
        NotificationKind::CiFailed => "CI failed".into(),
        NotificationKind::Due => if n.reason == "issueReminder" { "reminder" } else { "due" }.into(),
        NotificationKind::Comment => match who {
            Some(a) => format!("{a} commented"),
            None => "new comment".into(),
        },
        NotificationKind::StateChange => match n.excerpt.as_deref().filter(|s| !s.is_empty()) {
            Some(state) => format!("→ {state}"),
            None => "state changed".into(),
        },
        NotificationKind::NewItem => {
            if n.context.is_empty() {
                "new".into()
            } else {
                format!("new in {}", n.context)
            }
        }
        NotificationKind::Reaction => match who {
            Some(a) => format!("{a} reacted"),
            None => "reaction".into(),
        },
        // A finished workflow run that didn't fail (GitHub `ci_activity`): how it ended.
        NotificationKind::Other if n.reason == "ci_activity" => format!("CI {}", n.excerpt.as_deref().unwrap_or("finished")),
        NotificationKind::Other => match known_other(&n.reason) {
            Some(p) => p.into(),
            None if n.reason.is_empty() => "update".into(),
            None => humanize(&n.reason),
        },
    }
}

/// Why this reached you — the line under the preview's heading, so a reader can tell which
/// subscription to drop.
pub fn why(n: &Notification) -> String {
    let noun = item_noun(n.item_type);
    match n.kind {
        NotificationKind::Mention if n.reason == "team_mention" => "your team was @-mentioned".into(),
        NotificationKind::Mention => "you were @-mentioned".into(),
        NotificationKind::ReviewRequested => "your review was requested".into(),
        NotificationKind::Assigned => format!("this {noun} is assigned to you"),
        NotificationKind::CiFailed => format!("a check failed on this {noun}"),
        NotificationKind::Due if n.reason == "issueReminder" => "you set a reminder".into(),
        NotificationKind::Due => format!("this {noun} is due"),
        NotificationKind::NewItem if !n.context.is_empty() => format!("you follow {}", n.context),
        NotificationKind::Other => match n.reason.as_str() {
            "author" => format!("you opened this {noun}"),
            "ci_activity" => "a run you triggered finished".into(),
            "manual" => format!("you subscribed to this {noun}"),
            "subscribed" if !n.context.is_empty() => format!("you watch {}", n.context),
            _ => format!("you follow this {noun}"),
        },
        _ => format!("you follow this {noun}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NotificationTarget;
    use chrono::TimeZone;

    fn n(id: &str, kind: NotificationKind, item: Option<&str>, hour: u32, unread: bool) -> Notification {
        Notification {
            id: id.into(),
            kind,
            item_type: NotificationItemType::WorkItem,
            item_id: item.map(Into::into),
            repository: None,
            title: "t".into(),
            context: "Platform".into(),
            url: None,
            unread,
            updated_at: Some(Utc.with_ymd_and_hms(2026, 10, 15, hour, 0, 0).unwrap()),
            reason: String::new(),
            actor: None,
            excerpt: None,
            target: NotificationTarget::Overview,
            detail_ref: None,
        }
    }

    #[test]
    fn events_on_one_item_fold_into_one_thread_led_by_the_strongest_reason() {
        let ns = [
            n("1", NotificationKind::NewItem, Some("PLT-77"), 8, true),
            n("2", NotificationKind::Comment, Some("PLT-77"), 10, true),
            n("3", NotificationKind::Mention, Some("PLT-77"), 9, true),
            n("4", NotificationKind::Assigned, Some("ENG-1"), 11, true),
        ];
        let rows: Vec<(&str, &Notification)> = ns.iter().map(|x| ("linear", x)).collect();
        let t = threads(&rows);
        assert_eq!(t.len(), 2, "{t:?}");
        // ENG-1's event is the newest, so its thread sorts first.
        assert_eq!(rows[t[0].lead].1.id, "4");
        let plt = &t[1];
        assert_eq!(rows[plt.lead].1.id, "3", "the mention leads, though the comment is newer");
        assert_eq!(plt.tier, NotificationTier::ForYou);
        assert_eq!(plt.members.iter().map(|&i| rows[i].1.id.as_str()).collect::<Vec<_>>(), ["2", "3", "1"]);
        assert_eq!(plt.more(&rows), 2);
    }

    #[test]
    fn the_same_item_id_on_another_connection_or_repository_is_another_thread() {
        let mut a = n("1", NotificationKind::Comment, Some("7"), 8, true);
        let mut b = n("2", NotificationKind::Comment, Some("7"), 9, true);
        a.repository = Some("acme/pay".into());
        b.repository = Some("acme/web".into());
        let c = n("3", NotificationKind::Comment, Some("7"), 10, true);
        let rows = [("gh", &a), ("gh", &b), ("gl", &c)];
        assert_eq!(threads(&rows).len(), 3);
    }

    #[test]
    fn the_inbox_lists_only_what_waits_on_you() {
        let ns = [
            n("1", NotificationKind::Mention, Some("A"), 8, true),
            n("2", NotificationKind::Comment, Some("B"), 9, true),
            n("3", NotificationKind::NewItem, Some("C"), 10, true),
            n("4", NotificationKind::Assigned, Some("D"), 11, false),
        ];
        let rows: Vec<(&str, &Notification)> = ns.iter().map(|x| ("c", x)).collect();
        let ids: Vec<&str> = inbox(&rows).iter().map(|t| rows[t.lead].1.id.as_str()).collect();
        assert_eq!(ids, ["4", "1"], "read or not, only For you; newest first");
    }

    #[test]
    fn notifications_without_an_item_never_merge() {
        let a = n("1", NotificationKind::Other, None, 8, true);
        let b = n("2", NotificationKind::Other, None, 9, true);
        assert_eq!(threads(&[("gh", &a), ("gh", &b)]).len(), 2);
    }

    #[test]
    fn a_read_mention_gives_the_lead_to_an_unread_comment() {
        let ns = [
            n("1", NotificationKind::Mention, Some("X"), 8, false),
            n("2", NotificationKind::Comment, Some("X"), 9, true),
        ];
        let rows: Vec<(&str, &Notification)> = ns.iter().map(|x| ("c", x)).collect();
        let t = &threads(&rows)[0];
        assert_eq!(rows[t.lead].1.id, "2", "what's new leads the row");
        assert_eq!(t.tier, NotificationTier::ForYou, "but it is still the thread that mentioned you");
        assert!(t.unread);
    }

    /// Opening a thread reads all of it; it must still be in the inbox when you come back.
    #[test]
    fn reading_a_thread_never_changes_its_tier() {
        let mut ns = [n("1", NotificationKind::Mention, Some("X"), 8, true), n("2", NotificationKind::Comment, Some("X"), 9, true)];
        let tier_of = |ns: &[Notification]| {
            let rows: Vec<(&str, &Notification)> = ns.iter().map(|x| ("c", x)).collect();
            threads(&rows)[0].tier
        };
        assert_eq!(tier_of(&ns), NotificationTier::ForYou);
        ns.iter_mut().for_each(|x| x.unread = false);
        assert_eq!(tier_of(&ns), NotificationTier::ForYou);
        let rows: Vec<(&str, &Notification)> = ns.iter().map(|x| ("c", x)).collect();
        assert_eq!(rows[threads(&rows)[0].lead].1.id, "1", "read, the mention leads again");
    }

    #[test]
    fn a_detail_fills_only_what_the_feed_left_out() {
        let mut m = n("1", NotificationKind::Mention, None, 8, true);
        let d = crate::domain::NotificationDetail {
            actor: Some("Priya Nair".into()),
            excerpt: Some("ping".into()),
            target: Some(NotificationTarget::Comment { id: "7".into() }),
            url: None,
        };
        d.fill(&mut m);
        assert_eq!((m.actor.as_deref(), m.excerpt.as_deref()), (Some("Priya Nair"), Some("ping")));
        assert_eq!(m.target, NotificationTarget::Comment { id: "7".into() });
        // A target the feed already settled stays.
        let mut c = n("2", NotificationKind::CiFailed, None, 8, true);
        c.target = NotificationTarget::Checks;
        d.fill(&mut c);
        assert_eq!(c.target, NotificationTarget::Checks);
    }

    #[test]
    fn reactions_fold_into_their_thread_but_never_lead_count_or_stand_alone() {
        let ns = [
            n("1", NotificationKind::Reaction, Some("X"), 10, true),
            n("2", NotificationKind::Comment, Some("X"), 9, false),
            n("3", NotificationKind::Reaction, Some("Y"), 9, true),
        ];
        let rows: Vec<(&str, &Notification)> = ns.iter().map(|x| ("c", x)).collect();
        let t = threads(&rows);
        assert_eq!(t.len(), 1, "a reaction-only thread is left out");
        assert_eq!(rows[t[0].lead].1.id, "2");
        assert!(!t[0].unread, "an unread reaction doesn't make the thread unread");
        assert_eq!(t[0].more(&rows), 0);
        assert_eq!(t[0].members.len(), 2, "done still marks the reaction read");
    }

    #[test]
    fn every_kind_has_a_phrase_and_none_of_them_is_activity() {
        use NotificationKind::*;
        for kind in [ReviewRequested, Mention, Assigned, CiFailed, Comment, StateChange, NewItem, Due, Reaction, Other] {
            let p = phrase(&n("1", kind, None, 8, true));
            assert!(!p.is_empty() && p != "activity", "{kind:?} → {p}");
        }
    }

    #[test]
    fn phrases_name_the_actor_the_state_and_the_team() {
        let mut m = n("1", NotificationKind::Mention, None, 8, true);
        m.actor = Some("Priya Nair".into());
        assert_eq!(phrase(&m), "Priya mentioned you");
        let mut s = n("1", NotificationKind::StateChange, None, 8, true);
        s.excerpt = Some("In Review".into());
        assert_eq!(phrase(&s), "→ In Review");
        assert_eq!(phrase(&n("1", NotificationKind::NewItem, None, 8, true)), "new in Platform");
    }

    #[test]
    fn unknown_reasons_read_as_words() {
        let mut o = n("1", NotificationKind::Other, None, 8, true);
        o.reason = "issuePriorityUrgent".into();
        assert_eq!(phrase(&o), "marked urgent");
        o.reason = "issueTitleChanged".into();
        assert_eq!(phrase(&o), "title changed");
        o.reason = "member_access_requested".into();
        assert_eq!(phrase(&o), "member access requested");
        o.reason = String::new();
        assert_eq!(phrase(&o), "update");
        o.reason = "ci_activity".into();
        o.excerpt = Some("succeeded".into());
        assert_eq!(phrase(&o), "CI succeeded");
        assert_eq!(why(&o), "a run you triggered finished");
    }

    #[test]
    fn why_names_the_subscription() {
        assert_eq!(why(&n("1", NotificationKind::NewItem, None, 8, true)), "you follow Platform");
        let mut t = n("1", NotificationKind::Mention, None, 8, true);
        t.reason = "team_mention".into();
        assert_eq!(why(&t), "your team was @-mentioned");
        let mut a = n("1", NotificationKind::Other, None, 8, true);
        a.item_type = NotificationItemType::PullRequest;
        a.reason = "author".into();
        assert_eq!(why(&a), "you opened this pull request");
    }

    #[test]
    fn tiers_split_what_needs_you_from_what_you_follow() {
        use NotificationKind::*;
        for k in [ReviewRequested, Mention, Assigned, CiFailed, Due] {
            assert_eq!(k.tier(), NotificationTier::ForYou, "{k:?}");
        }
        for k in [Comment, StateChange, NewItem, Other] {
            assert_eq!(k.tier(), NotificationTier::Following, "{k:?}");
        }
        assert_eq!(Reaction.tier(), NotificationTier::Hidden);
    }

    #[test]
    fn a_dismissal_key_changes_when_the_thread_moves() {
        let a = n("9", NotificationKind::Comment, None, 8, true);
        let mut b = a.clone();
        b.updated_at = Some(Utc.with_ymd_and_hms(2026, 10, 15, 9, 0, 0).unwrap());
        assert_ne!(dismiss_key("gh", &a), dismiss_key("gh", &b));
        assert!(dismiss_key("gh", &a).starts_with("gh:9:"));
    }

    #[test]
    fn a_cached_notification_without_the_new_fields_still_loads() {
        let old = r#"{"id":"1","kind":"Comment","item_type":"PullRequest","item_id":"7","title":"t",
            "context":"c","url":null,"unread":true,"updated_at":null}"#;
        let n: Notification = serde_json::from_str(old).unwrap();
        assert_eq!(n.target, NotificationTarget::Overview);
        assert!(n.reason.is_empty() && n.actor.is_none() && n.detail_ref.is_none());
    }

    #[test]
    fn a_comment_target_serializes_as_a_tagged_object() {
        let v = serde_json::to_value(NotificationTarget::Comment { id: "42".into() }).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "Comment", "id": "42" }));
        assert_eq!(serde_json::to_value(NotificationTarget::Checks).unwrap(), serde_json::json!({ "kind": "Checks" }));
    }
}
