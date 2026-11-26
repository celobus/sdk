//! The charts catalogue: every `(family, kind)` pair the `viz` primitive draws (catalog revision 2). The list is
//! generated from the design repository's engine (`design/design/scripts/viz/`, one renderer per kind); the validator
//! reads it ([`known`]), and the SDKs build one constructor per kind from it: the Rust SDK through
//! [`viz_catalogue!`](crate::viz_catalogue), the TypeScript and C# SDKs from the files the test at the end of this
//! file keeps equal to the list (`sdk/node/src/viz-kinds.ts`, `sdk/dotnet/Celobus.Sdk/Ui/VizKinds.cs`).
//!
//! To change the catalogue: edit the list in `viz_catalogue!`, then run
//! `UPDATE_VIZ_KINDS=1 cargo test -p celobus-descriptor viz_kinds` to rewrite the two SDK files.

/// The catalogue as tokens, handed to the macro `$with`:
/// `$with! { family { kind, kind, snake_name "wireName" } … }`. A kind whose name in the document is not a
/// snake_case identifier carries that name as a string after its identifier (`meter_list "meterList"`).
#[macro_export]
macro_rules! viz_catalogue {
    ($with:path) => {
        $with! {
            bar { bar, column, diverging, dotplot, funnel, waterfall }
            dist { candles, correlation, cumulative, density, densitygrid, histogram, parallel, scatter, spread }
            figure { chips, hero, metric, metrics, stats, versus }
            flow {
                funnel, icicle, layers, matrix, network, outline, paths, polar, quadrant, radar, route, sankey,
                steps, tilemap, tree, words
            }
            geo { map }
            line { bump, line, multiples, slope }
            meter {
                bullet, checklist, gauge, level, meter, meter_list "meterList", range, rating, ring, steps, versus
            }
            part {
                budget, donut, onein, overlap, pictogram, rose, share, sharetile, split, squares, treemap, waffle
            }
            table { chart, grid, list, pivot }
            time {
                burn, calendar, cohort, countdown, cycle, heatmap, retention, schedule, sla, slots, streak, strip,
                timeline, updown
            }
        }
    };
}

/// A kind's name in the document: its identifier, or the string written after it.
#[doc(hidden)]
#[macro_export]
macro_rules! viz_kind_name {
    ($kind:ident) => {
        stringify!($kind)
    };
    ($kind:ident $name:literal) => {
        $name
    };
}

macro_rules! table {
    ($($family:ident { $($kind:ident $($name:literal)?),* $(,)? })*) => {
        /// Every family with its kinds, as the document names them (both sorted).
        pub const FAMILIES: &[(&str, &[&str])] = &[
            $((stringify!($family), &[$($crate::viz_kind_name!($kind $($name)?)),*])),*
        ];
    };
}
viz_catalogue!(table);

/// The kinds that show a second period of time travel (`time.then`) with an option of their own for "the period
/// before" (bars paired with it, a ghost of the columns, a delta on a metric, an inner ring of a donut, the change
/// of a share in points, two series of a funnel, a dumbbell, a treemap shaded by change, a change column of a
/// grid, the earlier period's line, quiet, under a line, a map's change between the two dates).
pub const COMPARE_KINDS: &[(&str, &str)] = &[
    ("bar", "bar"),
    ("bar", "column"),
    ("bar", "dotplot"),
    ("bar", "funnel"),
    ("figure", "metric"),
    ("figure", "metrics"),
    ("geo", "map"),
    ("line", "line"),
    ("part", "donut"),
    ("part", "share"),
    ("part", "treemap"),
    ("table", "grid"),
];

/// The kinds whose own axis is time: with the control `axis` a click on a mark moves the shown period.
pub const AXIS_KINDS: &[(&str, &str)] = &[("bar", "column"), ("line", "line")];

/// Whether the catalogue has this kind in this family.
pub fn known(family: &str, kind: &str) -> bool {
    kinds(family).is_some_and(|k| k.contains(&kind))
}

/// The kinds of a family (none: no such family).
pub fn kinds(family: &str) -> Option<&'static [&'static str]> {
    FAMILIES
        .iter()
        .find(|(f, _)| *f == family)
        .map(|(_, kinds)| *kinds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_is_sorted_and_whole() {
        assert_eq!(FAMILIES.len(), 10);
        assert_eq!(FAMILIES.iter().map(|(_, k)| k.len()).sum::<usize>(), 83);
        for (f, k) in COMPARE_KINDS.iter().chain(AXIS_KINDS) {
            assert!(known(f, k), "{f}.{k}");
        }
        assert!(FAMILIES.windows(2).all(|w| w[0].0 < w[1].0));
        for (family, kinds) in FAMILIES {
            assert!(kinds.windows(2).all(|w| w[0] < w[1]), "{family}");
        }
        assert!(known("line", "line") && known("meter", "meterList") && known("flow", "funnel"));
        assert!(!known("line", "column") && !known("pie", "pie") && !known("meter", "meter_list"));
    }

    /// `metric` → `Metric`, `meterList` → `MeterList`.
    fn pascal(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    fn typescript() -> String {
        let mut out = String::from(
            "// Generated from crates/descriptor/src/viz_kinds.rs; do not edit.\n\
             // UPDATE_VIZ_KINDS=1 cargo test -p celobus-descriptor viz_kinds\n\n\
             /** The charts catalogue: every kind the `viz` primitive draws, by family. */\n\
             export const VIZ_KINDS = {\n",
        );
        for (family, kinds) in FAMILIES {
            let list: Vec<String> = kinds.iter().map(|k| format!("\"{k}\"")).collect();
            out += &format!("  {family}: [{}],\n", list.join(", "));
        }
        out + "} as const;\n"
    }

    fn csharp() -> String {
        let mut out = String::from(
            "// Generated from crates/descriptor/src/viz_kinds.rs; do not edit.\n\
             // UPDATE_VIZ_KINDS=1 cargo test -p celobus-descriptor viz_kinds\n\n\
             namespace Celobus.Sdk.Ui;\n\n\
             /// <summary>The charts catalogue: one constructor per kind the <c>viz</c> primitive draws, by family\n\
             /// (<c>Viz.Line.Line()</c>, <c>Viz.Bar.Column()</c>, <c>Viz.Figure.Metric()</c>).</summary>\n\
             public static class Viz\n{\n",
        );
        for (family, _) in FAMILIES {
            let f = pascal(family);
            out += &format!("    public static {f}Kinds {f} {{ get; }} = new();\n");
        }
        out += "\n    /// <summary>Every family with its kinds, as the document names them.</summary>\n";
        out += "    public static readonly IReadOnlyDictionary<string, string[]> Kinds = new Dictionary<string, string[]>\n    {\n";
        for (family, kinds) in FAMILIES {
            let list: Vec<String> = kinds.iter().map(|k| format!("\"{k}\"")).collect();
            out += &format!("        [\"{family}\"] = [{}],\n", list.join(", "));
        }
        out += "    };\n}\n";
        for (family, kinds) in FAMILIES {
            let f = pascal(family);
            out += &format!(
                "\n/// <summary>The kinds of the <c>{family}</c> family.</summary>\npublic sealed class {f}Kinds\n{{\n    \
                 internal {f}Kinds() {{ }}\n\n"
            );
            for k in *kinds {
                out += &format!(
                    "    public VizNode {}() => new(\"{family}\", \"{k}\");\n",
                    pascal(k)
                );
            }
            out += "}\n";
        }
        out
    }

    /// The SDK files generated from the catalogue are up to date (`UPDATE_VIZ_KINDS=1` rewrites them).
    #[test]
    fn viz_kinds_of_the_sdks_follow_the_catalogue() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (path, want) in [
            ("sdk/node/src/viz-kinds.ts", typescript()),
            ("sdk/dotnet/Celobus.Sdk/Ui/VizKinds.cs", csharp()),
        ] {
            let file = root.join(path);
            if std::env::var_os("UPDATE_VIZ_KINDS").is_some() {
                std::fs::write(&file, &want).unwrap();
            }
            let got = std::fs::read_to_string(&file).unwrap_or_default();
            assert!(
                got == want,
                "{path} does not follow the catalogue: run UPDATE_VIZ_KINDS=1 cargo test -p celobus-descriptor viz_kinds"
            );
        }
    }
}
