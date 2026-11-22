// An entity's access rules (`entity({ access: (a) => { ... } })`): who reads, creates, changes and deletes its rows,
// who may change which column to what (guards), which columns only some roles read (masks), and what other plugins
// may inherit with `via` (export). The platform enforces them on every path; the SDK only
// describes them. Several grants given together are any-of; `a.all(..)` and
// `a.not(..)` combine them otherwise. Undeclared means denied: an entity without rules is read and written by no one but a
// system context (`ctx.asSystem(reason)`, within the plugin's `authz:bypass`).
//
//   access: (a) => {
//     const owner = a.relation('owner', a.column('ownerId'));
//     a.read(owner, manager);
//     a.update(owner.when({ status: 'draft' }), manager);   // also a list, null, a.ne(v), a.isNotNull()
//     a.permit('publish', manager);
//     a.guard('status').to('published', a.holds('publish'));
//     a.mask('secret', [manager]);
//   }
import type { RoleDef } from "./declare.js";
import { snake } from "./names.js";
import type { EntityDef, Shape } from "./schema.js";

/** A role as a grant: a plugin role, or `workspace.admin` and the other base roles. */
export type RoleLike = RoleDef | string;

/** @internal What a grant needs when it is described: the names of the plugin's roles and entities. */
export interface Resolve {
  role(r: RoleLike): string | undefined;
  /** An entity's name: its own (one of the plugin's), `<plugin>.<name>` (a dependency's), undefined (not declared). */
  entity?(e: EntityDef<any>): string | undefined;
  /** A problem of the rules (`code`: `E_ACCESS_ROLE_UNKNOWN` unless named). */
  error(message: string, code?: string): void;
}

/** A `when` filter as access conditions. */
function conditions(filter: Record<string, unknown>): unknown[] {
  return Object.entries(filter).map(([k, v]) =>
    v instanceof WhenTest
      ? { field: snake(k), op: v.op, value: v.value }
      : v === null
        ? { field: snake(k), op: "is_null", value: null }
        : Array.isArray(v)
          ? { field: snake(k), op: "in", value: v }
          : { field: snake(k), op: "eq", value: v },
  );
}

/** A grant: who passes. */
export class Grant {
  constructor(/** @internal */ readonly build: (r: Resolve) => unknown) {}

  /**
   * This grant, on the rows whose own fields equal these values (a list: any of them; `null`: none; `a.ne(v)`: not
   * `v`; `a.isNotNull()`: any value).
   */
  when(filter: Record<string, unknown>): Grant {
    return new Grant((r) => ({ when: { grant: this.build(r), filter: conditions(filter) } }));
  }
}

/** A test of a field in `when` other than an equality: `a.ne(v)`, `a.isNotNull()`. */
export class WhenTest {
  constructor(
    /** @internal */ readonly op: "ne" | "is_not_null",
    /** @internal */ readonly value: unknown,
  ) {}
}

/** A relation of the entity's rows: a grant by itself, or `.via(perm)` for a relation to a row. */
export class Relation extends Grant {
  constructor(readonly name: string) {
    super(() => ({ rel: name }));
  }

  /** The caller holds `perm` (`read`, `update`, `delete` or an action) on the row this relation points to. */
  via(perm: string): Grant {
    return new Grant(() => ({ via: { rel: this.name, perm } }));
  }
}

/** What a relation names: a column of a person or a row, a list of people, a group, the rows of a junction. */
export interface Rel {
  kind: "column" | "group" | "rows";
  field: string;
  /** @internal `rows`: the junction, its column that points to the row, the filter on its rows. */
  rows?: { entity: EntityDef<any>; target: string; filter: unknown[] };
}

/**
 * A relation through the rows of a junction entity (`a.rows(channelMember, 'channelId', 'userId')`); `.when(filter)`
 * counts only the junction rows whose own fields match.
 */
export class RowsRel implements Rel {
  readonly kind = "rows" as const;
  readonly field: string;
  /** @internal */ readonly rows: { entity: EntityDef<any>; target: string; filter: unknown[] };

  constructor(entity: EntityDef<any>, target: string, subject: string, filter: unknown[] = []) {
    this.field = snake(subject);
    this.rows = { entity, target: snake(target), filter };
  }

  /** Only the junction rows whose own fields match (as `Grant.when`: a value, a list, `null`, `a.ne(v)`, `a.isNotNull()`). */
  when(filter: Record<string, unknown>): RowsRel {
    return new RowsRel(this.rows.entity, this.rows.target, this.field, [...this.rows.filter, ...conditions(filter)]);
  }
}

export type GrantLike = Grant | RoleLike;

function grantOf(g: GrantLike): Grant {
  if (g instanceof Grant) return g;
  return new Grant((r) => {
    const name = r.role(g);
    if (name === undefined) r.error("a role of the rules is not in plugin({ roles })");
    return { role: name ?? "" };
  });
}

function anyOf(grants: GrantLike[]): Grant {
  if (grants.length === 1) return grantOf(grants[0]!);
  return new Grant((r) => ({ any: grants.map((g) => grantOf(g).build(r)) }));
}


/** The rules of a guard: `.to(value, ...grants)`, `.toAny(...grants)`. */
export class Guard {
  /** @internal */ readonly rules: { value: unknown; grant: Grant }[] = [];

  constructor(/** @internal */ readonly fields: string[]) {}

  /** To `value`, by whoever the grants admit (evaluated on the row as it was). */
  to(value: unknown, ...grants: GrantLike[]): this {
    this.rules.push({ value, grant: anyOf(grants) });
    return this;
  }

  /** To any value. */
  toAny(...grants: GrantLike[]): this {
    this.rules.push({ value: null, grant: anyOf(grants) });
    return this;
  }
}

/** What `access: (a) => ..` declares with. */
export class AccessBuilder {
  /** @internal */ readonly relations: (Rel & { name: string })[] = [];
  /** @internal */ readonly grants: Partial<Record<"read" | "create" | "update" | "delete", Grant>> = {};
  /** @internal */ readonly permits: { action: string; grant: Grant }[] = [];
  /** @internal */ readonly guards: Guard[] = [];
  /** @internal */ readonly masks: { field: string; roles: RoleLike[] }[] = [];
  /** @internal */ readonly exports: string[] = [];
  /** @internal */ admin = true;

  /** A column naming a person, or a row of an entity whose permissions `via` inherits. */
  column(key: string): Rel {
    return { kind: "column", field: snake(key) };
  }

  /** A group: its members. */
  group(key: string): Rel {
    return { kind: "group", field: snake(key) };
  }

  /**
   * The rows of the junction entity `junction` (one of this plugin's) whose `target` column points to the row: each
   * makes the person (`t.ref(platform.user)`) or the group (`t.ref(platform.group)`: its members) its `subject` column
   * names a subject of the relation, as the junction's rows are when the statement runs.
   */
  rows<C extends Shape>(junction: EntityDef<C>, target: keyof C & string, subject: keyof C & string): RowsRel {
    return new RowsRel(junction, target, subject);
  }

  relation(name: string, rel: Rel): Relation {
    this.relations.push({ name, kind: rel.kind, field: rel.field, ...(rel.rows ? { rows: rel.rows } : {}) });
    return new Relation(name);
  }

  read(...grants: GrantLike[]): void {
    this.grants.read = anyOf(grants);
  }

  /** Checks the new row (an owner relation must name the caller). */
  create(...grants: GrantLike[]): void {
    this.grants.create = anyOf(grants);
  }

  /** Checks the row as it was; the row as it becomes must stay readable to the writer. */
  update(...grants: GrantLike[]): void {
    this.grants.update = anyOf(grants);
  }

  delete(...grants: GrantLike[]): void {
    this.grants.delete = anyOf(grants);
  }

  /** A custom action (`publish`) and its grant: what `holds` names. */
  permit(action: string, ...grants: GrantLike[]): void {
    this.permits.push({ action, grant: anyOf(grants) });
  }

  /** All of the grants. */
  all(...grants: GrantLike[]): Grant {
    return new Grant((r) => ({ all: grants.map((g) => grantOf(g).build(r)) }));
  }

  /** Any of the grants (what several grants given together mean). */
  any(...grants: GrantLike[]): Grant {
    return new Grant((r) => ({ any: grants.map((g) => grantOf(g).build(r)) }));
  }

  /** Not the grant. */
  not(grant: GrantLike): Grant {
    return new Grant((r) => ({ not: grantOf(grant).build(r) }));
  }

  /** In `when`: the field is not `value` (a null is not equal to a value). */
  ne(value: unknown): WhenTest {
    return new WhenTest("ne", value);
  }

  /** In `when`: the field has a value. */
  isNotNull(): WhenTest {
    return new WhenTest("is_not_null", null);
  }

  /** Whoever the grant of a custom action admits. */
  holds(action: string): Grant {
    return new Grant(() => ({ holds: action }));
  }

  /** Everyone, members or not. */
  public(): Grant {
    return new Grant(() => "public");
  }

  /** Every member. */
  authenticated(): Grant {
    return new Grant(() => "authenticated");
  }

  /** Who may change a column (or any of several), on update. */
  guard(fields: string | string[]): Guard {
    const g = new Guard((Array.isArray(fields) ? fields : [fields]).map(snake));
    this.guards.push(g);
    return g;
  }

  /** Only these roles read the column (an optional one: others read it as absent). */
  mask(field: string, roles: RoleLike[]): void {
    this.masks.push({ field: snake(field), roles });
  }

  /** Permissions other plugins may inherit with `via`. */
  export(perms: string[]): void {
    this.exports.push(...perms);
  }

  /** `workspace.admin` stands in for the roles that are not strict (the default); `false` for rows admins must not see. */
  adminBypass(yes: boolean): void {
    this.admin = yes;
  }
}

/** @internal The rules as the descriptor carries them. */
export function describeAccess(declare: (a: AccessBuilder) => void, r: Resolve): Record<string, unknown> {
  const a = new AccessBuilder();
  declare(a);
  const grant = (g: Grant | undefined) => (g ? g.build(r) : null);
  return {
    relations: a.relations.map((x) => {
      const out: Record<string, unknown> = { name: x.name, kind: x.kind, field: x.field };
      if (x.rows) {
        const entity = r.entity?.(x.rows.entity);
        if (entity === undefined) {
          r.error(`the junction of relation ${x.name} is not an entity of the plugin`, "E_ACCESS_ROWS_PLUGIN");
        }
        out.rows = { entity: entity ?? "", target: x.rows.target, filter: x.rows.filter };
      }
      return out;
    }),
    read: grant(a.grants.read),
    create: grant(a.grants.create),
    update: grant(a.grants.update),
    delete: grant(a.grants.delete),
    permits: a.permits.map((p) => ({ action: p.action, grant: p.grant.build(r) })),
    guards: a.guards.map((g) => ({ fields: g.fields, rules: g.rules.map((x) => ({ value: x.value, grant: x.grant.build(r) })) })),
    masks: a.masks.map((m) => ({
      field: m.field,
      roles: m.roles.map((x) => {
        const name = r.role(x);
        if (name === undefined) r.error("a role of a mask is not in plugin({ roles })");
        return name ?? "";
      }),
    })),
    export: a.exports,
    admin: a.admin,
  };
}
