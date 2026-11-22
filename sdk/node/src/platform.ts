// The workspace directory, an implicit read-only dependency of every plugin:
//
//   import * as platform from '@celobus/sdk/platform';
//   ownerId: t.ref(platform.user)      // ref:platform.user
//
// and, for queries (`ctx.query(platform.groupMember).join(platform.user, platform.groupMember.f.userId)`), its
// people, groups, memberships and role assignments. A member (base role `member` and above) reads every row (a plugin:
// the assignments of its own roles and the platform's); a guest reads their own user row only, and sees others as
// labels (`label(ref)` from `@celobus/sdk/query`).
import { PlatformEntity, t, type Field } from "./schema.js";

/** A person or a service account. */
export const userKind = t.enum("UserKind", ["person", "service"]);
/** The base role in the workspace (`undefined` for a person who left). */
export const baseRole = t.enum("BaseRole", ["owner", "admin", "member", "guest"]);
/** Whether a person is in the workspace. */
export const userStatus = t.enum("UserStatus", ["active", "removed", "erased"]);

const userRef = (): Field<string, false, typeof user> => t.ref("platform.user") as never;
const groupRef = (): Field<string, false, typeof group> => t.ref("platform.group") as never;

/** A person (or service account) of the workspace. `email` is read only with the plugin permission `members.email`,
 * and never by a guest. */
export const user: PlatformEntity<{
  id: Field<string, false, PlatformEntity<any>>;
  displayName: Field<string, false>;
  avatarUrl: Field<string, false>;
  email: Field<string, true>;
  kind: Field<"person" | "service", false>;
  baseRole: Field<"owner" | "admin" | "member" | "guest", true>;
  status: Field<"active" | "removed" | "erased", false>;
  locale: Field<string, false>;
  joinedAt: Field<Date, false>;
  leftAt: Field<Date, true>;
}> = new PlatformEntity("platform.user", {
  id: t.ref("platform.user") as unknown as Field<string, false, PlatformEntity<any>>,
  displayName: t.text(),
  avatarUrl: t.text(),
  email: t.text().optional(),
  kind: t.enum(userKind),
  baseRole: t.enum(baseRole).optional(),
  status: t.enum(userStatus),
  locale: t.text(),
  joinedAt: t.timestamp(),
  leftAt: t.timestamp().optional(),
});

/** A group of the workspace. */
export const group: PlatformEntity<{ id: Field<string, false, PlatformEntity<any>>; name: Field<string, false> }> =
  new PlatformEntity("platform.group", {
    id: t.ref("platform.group") as unknown as Field<string, false, PlatformEntity<any>>,
    name: t.text(),
  });

/** A person's membership of a group (queries only). */
export const groupMember = new PlatformEntity("platform.group_member", {
  groupId: groupRef(),
  userId: userRef(),
});

/** A role a person holds: `workspace.*` and the plugin's own (queries only). */
export const roleAssignment = new PlatformEntity("platform.role_assignment", {
  userId: userRef(),
  role: t.text(),
});
