// The views of the conformance plugin `jobsdemo` (sdk/conformance/jobs/ts/src/index.ts), in JavaScript against the
// build, and the conformance queries over them (sdk/conformance/query/weekly.json, estimates.json).
import { Decimal, daysAgo, entity, plugin, t, view } from "../../dist/index.js";
import { count } from "../../dist/query.js";

export const task = entity({ columns: { title: t.text(), status: t.text() } });
export const stage = entity({ columns: { name: t.text(), title: t.text() } });
export const estimate = entity({ columns: { title: t.text(), hours: t.decimal() } });

export const recentTask = view({
  description: "The titled tasks of the last 30 days, a row each, with the title of their stage.",
  columns: { day: t.date(), title: t.text(), stage: t.text().optional() },
  query: (q) =>
    q
      .from(task)
      .leftJoinOn(stage, stage.f.name.eqCol(task.f.status))
      .where(task.f.createdAt.geCol(daysAgo(30)), task.f.title.ne(""))
      .columns({ day: task.f.createdAt.day(), title: task.f.title, stage: stage.f.title }),
});

export const tasksByWeek = view({
  description: "The recent tasks by week and stage, where a stage had more than one.",
  columns: { week: t.date(), stage: t.text().optional(), tasks: t.bigint() },
  query: (q) =>
    q
      .from(recentTask)
      .groupBy(recentTask.f.day.week(), recentTask.f.stage)
      .having(count().gt(1))
      .columns({ week: recentTask.f.day.week(), stage: recentTask.f.stage, tasks: count() }),
});

export const longEstimate = view({
  description: "The estimates of more than 12.50 hours.",
  columns: { title: t.text(), hours: t.decimal() },
  query: (q) =>
    q
      .from(estimate)
      .where(estimate.f.hours.gt(Decimal.from("12.50")))
      .columns({ title: estimate.f.title, hours: estimate.f.hours }),
});

export default plugin({
  id: "jobsdemo",
  version: "0.1.0",
  entities: { task, stage, estimate },
  views: { recentTask, tasksByWeek, longEstimate },
});

const r = recentTask.f;

/** The conformance query over the view `recent_task`: the recent tasks of the last week whose month falls in 2026,
 * counted by month and stage. */
export function weekly(q) {
  return q
    .where(r.day.geCol(daysAgo(7)), r.day.month().between("2026-01-01", "2026-12-01"))
    .groupBy(r.day.month(), r.stage)
    .select({ month: r.day.month(), stage: r.stage, tasks: count() })
    .orderBy(r.day.month().asc())
    .limit(12);
}

const e = longEstimate.f;

/** The conformance query over the view `long_estimate`: the estimates of at least 20.50 hours, longest first. */
export function estimates(q) {
  return q
    .where(e.hours.ge(Decimal.from("20.50")))
    .select({ title: e.title, hours: e.hours })
    .orderBy(e.hours.desc());
}
