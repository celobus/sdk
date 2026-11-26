// Generated from crates/descriptor/src/viz_kinds.rs; do not edit.
// UPDATE_VIZ_KINDS=1 cargo test -p celobus-descriptor viz_kinds

/** The charts catalogue: every kind the `viz` primitive draws, by family. */
export const VIZ_KINDS = {
  bar: ["bar", "column", "diverging", "dotplot", "funnel", "waterfall"],
  dist: ["candles", "correlation", "cumulative", "density", "densitygrid", "histogram", "parallel", "scatter", "spread"],
  figure: ["chips", "hero", "metric", "metrics", "stats", "versus"],
  flow: ["funnel", "icicle", "layers", "matrix", "network", "outline", "paths", "polar", "quadrant", "radar", "route", "sankey", "steps", "tilemap", "tree", "words"],
  geo: ["map"],
  line: ["bump", "line", "multiples", "slope"],
  meter: ["bullet", "checklist", "gauge", "level", "meter", "meterList", "range", "rating", "ring", "steps", "versus"],
  part: ["budget", "donut", "onein", "overlap", "pictogram", "rose", "share", "sharetile", "split", "squares", "treemap", "waffle"],
  table: ["chart", "grid", "list", "pivot"],
  time: ["burn", "calendar", "cohort", "countdown", "cycle", "heatmap", "retention", "schedule", "sla", "slots", "streak", "strip", "timeline", "updown"],
} as const;
