using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Terminal.Gui;

namespace Forgetop.Tui;

/// <summary>Pull Requests screen: (f)ilter, (a)pprove, (m)erge, (c)omment.</summary>
public sealed class PullRequestsView(PullRequestController controller) : SectionView("Pull Requests")
{
    protected override Task<SectionData> LoadAsync(CancellationToken ct = default) => controller.LoadAsync(ct);

    protected override bool OnActionKey(KeyEvent keyEvent)
    {
        switch (KeyChar(keyEvent))
        {
            case 'f':
                controller.CycleFilter();
                RunAction(() => Task.CompletedTask);
                return true;
            case 'a':
                RunAction(() => controller.VoteAsync(SelectedIndex, ReviewVote.Approved));
                return true;
            case 'm':
                if (Dialogs.Confirm("Merge", "Merge the selected pull request?"))
                {
                    RunAction(() => controller.MergeAsync(SelectedIndex, new MergeOptions()));
                }

                return true;
            case 'c':
                var body = Dialogs.Prompt("Comment", "Comment body:");
                if (body is not null)
                {
                    RunAction(() => controller.CommentAsync(SelectedIndex, body));
                }

                return true;
            default:
                return false;
        }
    }
}

/// <summary>Work Items screen: (s)tate change, (c)omment.</summary>
public sealed class WorkItemsView(WorkItemController controller) : SectionView("Work Items")
{
    protected override Task<SectionData> LoadAsync(CancellationToken ct = default) => controller.LoadAsync(ct);

    protected override bool OnActionKey(KeyEvent keyEvent)
    {
        switch (KeyChar(keyEvent))
        {
            case 's':
                var state = Dialogs.Prompt("Set state", "New state (e.g. open/closed, In Progress, Done):");
                if (state is not null)
                {
                    RunAction(() => controller.SetStateAsync(SelectedIndex, state));
                }

                return true;
            case 'c':
                var body = Dialogs.Prompt("Comment", "Comment body:");
                if (body is not null)
                {
                    RunAction(() => controller.CommentAsync(SelectedIndex, body));
                }

                return true;
            default:
                return false;
        }
    }
}

/// <summary>Pipelines screen: ↵ drill-in, (t)rigger, (d)iscover &amp; subscribe.</summary>
public sealed class PipelinesView(PipelineController controller) : SectionView("Pipelines")
{
    protected override Task<SectionData> LoadAsync(CancellationToken ct = default) => controller.LoadAsync(ct);

    protected override bool OnActionKey(KeyEvent keyEvent)
    {
        if (keyEvent.Key == Key.Enter)
        {
            var detail = controller.GetRunDetailAsync(SelectedIndex).GetAwaiter().GetResult();
            ShowDetail(detail);
            return true;
        }

        switch (KeyChar(keyEvent))
        {
            case 't':
                if (Dialogs.Confirm("Trigger", "Re-run the selected pipeline?"))
                {
                    RunAction(() => controller.TriggerAsync(SelectedIndex));
                }

                return true;
            case 'd':
                Discover();
                return true;
            default:
                return false;
        }
    }

    private void Discover()
    {
        var found = controller.DiscoverAsync().GetAwaiter().GetResult();
        if (found.Count == 0)
        {
            Dialogs.Info("Discover", "No pipelines found on the bound connections.");
            return;
        }

        var labels = found.Select(d => $"{d.ConnectionLabel}: {d.Definition.Name}").ToList();
        var picked = Dialogs.Pick("Subscribe to pipeline", labels);
        if (picked is not null)
        {
            var chosen = found[labels.IndexOf(picked)];
            RunAction(() => controller.SubscribeAsync(chosen));
        }
    }
}
