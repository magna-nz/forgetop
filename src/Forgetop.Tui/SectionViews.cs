using System.Data;
using Forgetop.Core.Domain;
using Forgetop.Core.Providers;
using Terminal.Gui;
using Terminal.Gui.Trees;

namespace Forgetop.Tui;

/// <summary>A node in the pipeline drill-in tree (stage → job → step).</summary>
internal sealed record PipeNode(string Label, IReadOnlyList<PipeNode> Children);

/// <summary>Pull Requests: Title · CI · ± · Created. Enter → overview; f/a/m/c/d/v actions.</summary>
public sealed class PullRequestsView(PullRequestController controller) : SectionView
{
    public override Task LoadDataAsync(CancellationToken ct = default) => controller.LoadAsync(ct);

    public override void Render()
    {
        SetHeader($"Pull Requests   ·   {controller.Label}");

        var items = controller.Items;
        var dt = new DataTable();
        dt.Columns.Add("Title");
        dt.Columns.Add("CI");
        dt.Columns.Add("±");
        dt.Columns.Add("Created");
        foreach (var pr in items)
        {
            dt.Rows.Add(
                $"#{pr.Number?.ToString() ?? pr.Id}  {pr.Title}",
                StatusColors.CheckIcon(pr.Checks),
                $"+{pr.Additions} -{pr.Deletions}",
                Fmt.Date(pr.CreatedAt));
        }

        SetTable(dt);
        Table.Style.ColumnStyles[dt.Columns["CI"]!] = new TableView.ColumnStyle
        {
            Alignment = TextAlignment.Centered,
            ColorGetter = a => StatusColors.Scheme(Table, a.RowIndex >= 0 && a.RowIndex < items.Count ? StatusColors.CheckColor(items[a.RowIndex].Checks) : Color.Gray),
        };
    }

    protected override void OnActivated(int row)
    {
        if (row >= 0 && row < controller.Items.Count)
        {
            Expand(DetailFormatter.PrOverview(controller.Items[row]));
        }
    }

    protected override bool OnActionKey(KeyEvent keyEvent)
    {
        switch (KeyChar(keyEvent))
        {
            case 'f':
                controller.CycleFilter();
                RunAction(() => Task.CompletedTask);
                return true;
            case 'a':
                RunAction(() => controller.VoteAsync(SelectedRow, ReviewVote.Approved));
                return true;
            case 'm':
                if (Dialogs.Confirm("Merge", "Merge the selected pull request?"))
                {
                    RunAction(() => controller.MergeAsync(SelectedRow, new MergeOptions()));
                }

                return true;
            case 'c':
                var body = Dialogs.Prompt("Comment", "Comment body:");
                if (body is not null)
                {
                    RunAction(() => controller.CommentAsync(SelectedRow, body));
                }

                return true;
            case 'd':
                ShowDetailSafe(() => controller.GetDiffTextAsync(SelectedRow));
                return true;
            case 'v':
                ShowDetailSafe(() => controller.GetThreadsTextAsync(SelectedRow));
                return true;
            default:
                return false;
        }
    }
}

/// <summary>Work Items: Id · Title · State · Type · Assignee. Enter → detail; f/s/c actions.</summary>
public sealed class WorkItemsView(WorkItemController controller) : SectionView
{
    public override Task LoadDataAsync(CancellationToken ct = default) => controller.LoadAsync(ct);

    public override void Render()
    {
        SetHeader($"Work Items   ·   {controller.Label}");

        var items = controller.Items;
        var dt = new DataTable();
        dt.Columns.Add("Id");
        dt.Columns.Add("Title");
        dt.Columns.Add("State");
        dt.Columns.Add("Type");
        dt.Columns.Add("Assignee");
        foreach (var w in items)
        {
            dt.Rows.Add(w.Identifier ?? w.Id, w.Title, w.State, w.Type ?? "-", w.Assignee?.DisplayName ?? "–");
        }

        SetTable(dt);
    }

    protected override void OnActivated(int row)
    {
        if (row >= 0 && row < controller.Items.Count)
        {
            Expand(DetailFormatter.WorkItemDetail(controller.Items[row]));
        }
    }

    protected override bool OnActionKey(KeyEvent keyEvent)
    {
        switch (KeyChar(keyEvent))
        {
            case 'f':
                controller.ToggleMine();
                RunAction(() => Task.CompletedTask);
                return true;
            case 's':
                var state = Dialogs.Prompt("Set state", "New state (e.g. open/closed, In Progress, Done):");
                if (state is not null)
                {
                    RunAction(() => controller.SetStateAsync(SelectedRow, state));
                }

                return true;
            case 'c':
                var body = Dialogs.Prompt("Comment", "Comment body:");
                if (body is not null)
                {
                    RunAction(() => controller.CommentAsync(SelectedRow, body));
                }

                return true;
            default:
                return false;
        }
    }
}

/// <summary>Pipelines: Status · Pipeline · Branch · Build · Timestamp · Duration. Enter → jobs; t/d/u actions.</summary>
public sealed class PipelinesView(PipelineController controller) : SectionView
{
    public override Task LoadDataAsync(CancellationToken ct = default) => controller.LoadAsync(ct);

    public override void Render()
    {
        SetHeader($"Pipelines   ·   {controller.Label}");

        var items = controller.Items;
        var dt = new DataTable();
        dt.Columns.Add("Status");
        dt.Columns.Add("Provider");
        dt.Columns.Add("Pipeline");
        dt.Columns.Add("Branch");
        dt.Columns.Add("Build");
        dt.Columns.Add("Timestamp");
        dt.Columns.Add("Duration");
        foreach (var (connection, run) in items)
        {
            dt.Rows.Add(
                StatusColors.PipelineLabel(run.Status),
                connection,
                run.Name ?? "–",
                run.Branch ?? "–",
                run.Number is { } n ? $"#{n}" : run.Id,
                Fmt.DateTime(run.StartedAt),
                Fmt.Duration(run));
        }

        SetTable(dt);
        Table.Style.ColumnStyles[dt.Columns["Status"]!] = new TableView.ColumnStyle
        {
            ColorGetter = a => StatusColors.Scheme(Table, a.RowIndex >= 0 && a.RowIndex < items.Count ? StatusColors.PipelineColor(items[a.RowIndex].Run.Status) : Color.Gray),
        };
    }

    protected override void OnActivated(int row)
    {
        if (row < 0 || row >= controller.Items.Count)
        {
            return;
        }

        try
        {
            var run = controller.GetRunAsync(row).GetAwaiter().GetResult();
            if (run is null)
            {
                return;
            }

            if (run.Stages.Count == 0)
            {
                Expand(controller.GetRunDetailAsync(row).GetAwaiter().GetResult());
                return;
            }

            ExpandView(BuildTree(run));
        }
        catch (Exception ex)
        {
            Dialogs.Error("forgetop", ex.Message);
        }
    }

    private static TreeView<PipeNode> BuildTree(PipelineRun run)
    {
        var stages = run.Stages.Select(stage => new PipeNode(
            $"{StatusColors.PipelineIcon(stage.Status)} {stage.Name}",
            stage.Jobs.Select(job => new PipeNode(
                $"{StatusColors.PipelineIcon(job.Status)} {job.Name}",
                job.Steps.Select(step => new PipeNode($"{StatusColors.PipelineIcon(step.Status)} {step.Name}", [])).ToList())).ToList())).ToList();

        var tree = new TreeView<PipeNode>
        {
            TreeBuilder = new DelegateTreeBuilder<PipeNode>(n => n.Children),
            AspectGetter = n => n.Label,
        };
        tree.AddObjects(stages);
        return tree;
    }

    protected override bool OnActionKey(KeyEvent keyEvent)
    {
        switch (KeyChar(keyEvent))
        {
            case 't':
                if (Dialogs.Confirm("Trigger", "Re-run the selected pipeline?"))
                {
                    RunAction(() => controller.TriggerAsync(SelectedRow));
                }

                return true;
            case 'd':
                Discover();
                return true;
            case 'u':
                if (Dialogs.Confirm("Unsubscribe", "Stop tracking the selected pipeline?"))
                {
                    RunAction(() => controller.UnsubscribeSelectedAsync(SelectedRow));
                }

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
