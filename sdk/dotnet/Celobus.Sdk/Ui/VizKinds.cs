// Generated from crates/descriptor/src/viz_kinds.rs; do not edit.
// UPDATE_VIZ_KINDS=1 cargo test -p celobus-descriptor viz_kinds

namespace Celobus.Sdk.Ui;

/// <summary>The charts catalogue: one constructor per kind the <c>viz</c> primitive draws, by family
/// (<c>Viz.Line.Line()</c>, <c>Viz.Bar.Column()</c>, <c>Viz.Figure.Metric()</c>).</summary>
public static class Viz
{
    public static BarKinds Bar { get; } = new();
    public static DistKinds Dist { get; } = new();
    public static FigureKinds Figure { get; } = new();
    public static FlowKinds Flow { get; } = new();
    public static GeoKinds Geo { get; } = new();
    public static LineKinds Line { get; } = new();
    public static MeterKinds Meter { get; } = new();
    public static PartKinds Part { get; } = new();
    public static TableKinds Table { get; } = new();
    public static TimeKinds Time { get; } = new();

    /// <summary>Every family with its kinds, as the document names them.</summary>
    public static readonly IReadOnlyDictionary<string, string[]> Kinds = new Dictionary<string, string[]>
    {
        ["bar"] = ["bar", "column", "diverging", "dotplot", "funnel", "waterfall"],
        ["dist"] = ["candles", "correlation", "cumulative", "density", "densitygrid", "histogram", "parallel", "scatter", "spread"],
        ["figure"] = ["chips", "hero", "metric", "metrics", "stats", "versus"],
        ["flow"] = ["funnel", "icicle", "layers", "matrix", "network", "outline", "paths", "polar", "quadrant", "radar", "route", "sankey", "steps", "tilemap", "tree", "words"],
        ["geo"] = ["map"],
        ["line"] = ["bump", "line", "multiples", "slope"],
        ["meter"] = ["bullet", "checklist", "gauge", "level", "meter", "meterList", "range", "rating", "ring", "steps", "versus"],
        ["part"] = ["budget", "donut", "onein", "overlap", "pictogram", "rose", "share", "sharetile", "split", "squares", "treemap", "waffle"],
        ["table"] = ["chart", "grid", "list", "pivot"],
        ["time"] = ["burn", "calendar", "cohort", "countdown", "cycle", "heatmap", "retention", "schedule", "sla", "slots", "streak", "strip", "timeline", "updown"],
    };
}

/// <summary>The kinds of the <c>bar</c> family.</summary>
public sealed class BarKinds
{
    internal BarKinds() { }

    public VizNode Bar() => new("bar", "bar");
    public VizNode Column() => new("bar", "column");
    public VizNode Diverging() => new("bar", "diverging");
    public VizNode Dotplot() => new("bar", "dotplot");
    public VizNode Funnel() => new("bar", "funnel");
    public VizNode Waterfall() => new("bar", "waterfall");
}

/// <summary>The kinds of the <c>dist</c> family.</summary>
public sealed class DistKinds
{
    internal DistKinds() { }

    public VizNode Candles() => new("dist", "candles");
    public VizNode Correlation() => new("dist", "correlation");
    public VizNode Cumulative() => new("dist", "cumulative");
    public VizNode Density() => new("dist", "density");
    public VizNode Densitygrid() => new("dist", "densitygrid");
    public VizNode Histogram() => new("dist", "histogram");
    public VizNode Parallel() => new("dist", "parallel");
    public VizNode Scatter() => new("dist", "scatter");
    public VizNode Spread() => new("dist", "spread");
}

/// <summary>The kinds of the <c>figure</c> family.</summary>
public sealed class FigureKinds
{
    internal FigureKinds() { }

    public VizNode Chips() => new("figure", "chips");
    public VizNode Hero() => new("figure", "hero");
    public VizNode Metric() => new("figure", "metric");
    public VizNode Metrics() => new("figure", "metrics");
    public VizNode Stats() => new("figure", "stats");
    public VizNode Versus() => new("figure", "versus");
}

/// <summary>The kinds of the <c>flow</c> family.</summary>
public sealed class FlowKinds
{
    internal FlowKinds() { }

    public VizNode Funnel() => new("flow", "funnel");
    public VizNode Icicle() => new("flow", "icicle");
    public VizNode Layers() => new("flow", "layers");
    public VizNode Matrix() => new("flow", "matrix");
    public VizNode Network() => new("flow", "network");
    public VizNode Outline() => new("flow", "outline");
    public VizNode Paths() => new("flow", "paths");
    public VizNode Polar() => new("flow", "polar");
    public VizNode Quadrant() => new("flow", "quadrant");
    public VizNode Radar() => new("flow", "radar");
    public VizNode Route() => new("flow", "route");
    public VizNode Sankey() => new("flow", "sankey");
    public VizNode Steps() => new("flow", "steps");
    public VizNode Tilemap() => new("flow", "tilemap");
    public VizNode Tree() => new("flow", "tree");
    public VizNode Words() => new("flow", "words");
}

/// <summary>The kinds of the <c>geo</c> family.</summary>
public sealed class GeoKinds
{
    internal GeoKinds() { }

    public VizNode Map() => new("geo", "map");
}

/// <summary>The kinds of the <c>line</c> family.</summary>
public sealed class LineKinds
{
    internal LineKinds() { }

    public VizNode Bump() => new("line", "bump");
    public VizNode Line() => new("line", "line");
    public VizNode Multiples() => new("line", "multiples");
    public VizNode Slope() => new("line", "slope");
}

/// <summary>The kinds of the <c>meter</c> family.</summary>
public sealed class MeterKinds
{
    internal MeterKinds() { }

    public VizNode Bullet() => new("meter", "bullet");
    public VizNode Checklist() => new("meter", "checklist");
    public VizNode Gauge() => new("meter", "gauge");
    public VizNode Level() => new("meter", "level");
    public VizNode Meter() => new("meter", "meter");
    public VizNode MeterList() => new("meter", "meterList");
    public VizNode Range() => new("meter", "range");
    public VizNode Rating() => new("meter", "rating");
    public VizNode Ring() => new("meter", "ring");
    public VizNode Steps() => new("meter", "steps");
    public VizNode Versus() => new("meter", "versus");
}

/// <summary>The kinds of the <c>part</c> family.</summary>
public sealed class PartKinds
{
    internal PartKinds() { }

    public VizNode Budget() => new("part", "budget");
    public VizNode Donut() => new("part", "donut");
    public VizNode Onein() => new("part", "onein");
    public VizNode Overlap() => new("part", "overlap");
    public VizNode Pictogram() => new("part", "pictogram");
    public VizNode Rose() => new("part", "rose");
    public VizNode Share() => new("part", "share");
    public VizNode Sharetile() => new("part", "sharetile");
    public VizNode Split() => new("part", "split");
    public VizNode Squares() => new("part", "squares");
    public VizNode Treemap() => new("part", "treemap");
    public VizNode Waffle() => new("part", "waffle");
}

/// <summary>The kinds of the <c>table</c> family.</summary>
public sealed class TableKinds
{
    internal TableKinds() { }

    public VizNode Chart() => new("table", "chart");
    public VizNode Grid() => new("table", "grid");
    public VizNode List() => new("table", "list");
    public VizNode Pivot() => new("table", "pivot");
}

/// <summary>The kinds of the <c>time</c> family.</summary>
public sealed class TimeKinds
{
    internal TimeKinds() { }

    public VizNode Burn() => new("time", "burn");
    public VizNode Calendar() => new("time", "calendar");
    public VizNode Cohort() => new("time", "cohort");
    public VizNode Countdown() => new("time", "countdown");
    public VizNode Cycle() => new("time", "cycle");
    public VizNode Heatmap() => new("time", "heatmap");
    public VizNode Retention() => new("time", "retention");
    public VizNode Schedule() => new("time", "schedule");
    public VizNode Sla() => new("time", "sla");
    public VizNode Slots() => new("time", "slots");
    public VizNode Streak() => new("time", "streak");
    public VizNode Strip() => new("time", "strip");
    public VizNode Timeline() => new("time", "timeline");
    public VizNode Updown() => new("time", "updown");
}
