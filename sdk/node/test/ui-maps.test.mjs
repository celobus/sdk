// UI catalog 4: time travel in a viz (`ui.time`) and maps (`ui.viz.geo.map()`, `ui.layer`) over a maps connection.
import assert from "node:assert/strict";
import { test } from "node:test";
import { endpoint, plugin, t, ui } from "../dist/index.js";

const quarter = t.message("Quarter", { key: t.text(), label: t.text(), total: t.decimal(), running: t.bool() });
const quartersRequest = t.message("QuartersRequest", {});
const quartersResponse = t.message("QuartersResponse", { quarters: t.array(quarter) });
const deal = t.message("Deal", { id: t.text(), name: t.text(), lat: t.double(), lon: t.double(), price: t.decimal(), ours: t.bool() });
const dealsRequest = t.message("DealsRequest", { quarter: t.text(), window: t.int(), area: t.text(), upto: t.int() });
const dealsResponse = t.message("DealsResponse", { deals: t.array(deal) });
const dealsParams = t.message("DealsParams", {
  quarter: t.text().optional(),
  window: t.int().optional(),
  then: t.text().optional(),
  area: t.text().optional(),
  upto: t.int().optional(),
});
const DEALS = ui.screenKey("deals", dealsParams);
const quarters = endpoint({ input: quartersRequest, output: quartersResponse, readOnly: true, handler: async () => ({ quarters: [] }) });
const deals = endpoint({ input: dealsRequest, output: dealsResponse, readOnly: true, handler: async () => ({ deals: [] }) });

const broker = plugin({
  id: "broker",
  version: "1.0.0",
  connections: { maps: { kind: "maps", title: "Maps" } },
  endpoints: { quarters: quarters.ui(), deals: deals.ui() },
  ui: ui.define({
    screens: [
      ui.screen(DEALS, (s) => {
        const q = s.param(dealsParams.f.quarter);
        const window = s.param(dealsParams.f.window);
        const then = s.param(dealsParams.f.then);
        const area = s.param(dealsParams.f.area);
        const upto = s.param(dealsParams.f.upto);
        const picked = s.state("picked", t.text(), "");
        const periods = s.query("quarters", ui.call(quarters, {}).rows(quartersResponse.f.quarters));
        const list = s.query("deals", ui.call(deals, { quarter: q, window, area, upto }).rows(dealsResponse.f.deals));
        return ui
          .board()
          .half(
            ui.viz.bar
              .bar()
              .title("Top deals")
              .rows(list, { name: deal.f.name, value: deal.f.price })
              .time(
                ui.time
                  .flow(periods, quarter.f.key, quarter.f.label)
                  .at(q)
                  .track(quarter.f.total)
                  .partial(quarter.f.running)
                  .window(window)
                  .compare(then)
                  .compareBy("previous")
                  .upto(upto)
                  .control("brush")
                  .grain("quarter"),
              ),
          )
          .half(
            ui.viz.geo
              .map()
              .title("Deals")
              .connection("maps")
              .layer(
                ui.layer
                  .points(list)
                  .lat(deal.f.lat)
                  .lon(deal.f.lon)
                  .value(deal.f.price)
                  .name(deal.f.name)
                  .id(deal.f.id)
                  .title("Deals")
                  .option("size", true)
                  .selectState(picked),
              )
              .layer(ui.layer.share(list).lat(deal.f.lat).lon(deal.f.lon).ours(deal.f.ours))
              .layer(ui.layer.area().title("Drawn area").selectParam(area))
              .time(ui.time.flow(periods, quarter.f.key, quarter.f.label).at(q)),
          );
      }),
    ],
  }),
});

test("time travel and maps describe as the contract writes them", () => {
  const d = broker.describe();
  assert.deepEqual(broker.check(), []);
  assert.equal(d.ui.catalog, 4);
  assert.deepEqual(d.connections, [{ name: "maps", kind: "maps", title: "Maps" }]);
  const items = d.ui.screens[0].layout.board.children;
  assert.deepEqual(items[0].node.viz.time, {
    read: "flow",
    control: "brush",
    periods: { query: "quarters", value: "key", label: "label", track: "total", partial: "running" },
    at: { param: "quarter" },
    window: { param: "window" },
    then: { param: "then" },
    compare: "previous",
    upto: { param: "upto" },
    grain: "quarter",
  });
  const map = items[1].node.viz;
  assert.deepEqual([map.family, map.kind, map.connection], ["geo", "map", "maps"]);
  assert.deepEqual(map.layers[0], {
    type: "points",
    query: "deals",
    fields: { lat: "lat", lon: "lon", value: "price", name: "name", id: "id" },
    title: "Deals",
    options: { size: true },
    select: { state: "picked" },
  });
  assert.deepEqual(map.layers[2], { type: "area", title: "Drawn area", select: { param: "area" } });
  assert.deepEqual(map.time, { read: "flow", periods: { query: "quarters", value: "key", label: "label" }, at: { param: "quarter" } });
});

test("time travel names the shown period", () => {
  assert.throws(() => ui.viz.bar.bar().time(ui.time.flow({ name: "q" }, { name: "key" }, { name: "label" })), /\.at\(\.\.\)/);
});
