// Name conversions between TypeScript keys and descriptor names.
//
// TypeScript declares fields, roles, scopes and entities with camelCase keys; the descriptor names them in
// snake_case. The wire uses the protobuf JSON name of a field (lowerCamelCase), which for a well-formed key is
// the key itself.

/** `ownerId` → `owner_id`. */
export function snake(key: string): string {
  return key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
}

/** The protobuf JSON name of a field: `owner_id` → `ownerId`. */
export function jsonName(name: string): string {
  let out = "";
  let upper = false;
  for (const c of name) {
    if (c === "_") {
      upper = true;
      continue;
    }
    out += upper ? c.toUpperCase() : c;
    upper = false;
  }
  return out;
}

/** `getNote` (or `get_note`) → `GetNote`. */
export function pascal(key: string): string {
  const camel = jsonName(key);
  return camel.charAt(0).toUpperCase() + camel.slice(1);
}

/** `hello` → `Hello`: the default title of a plugin. */
export function humanize(id: string): string {
  const words = snake(id).split("_").filter(Boolean).join(" ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The service of a plugin: its id in PascalCase (`orders` → `Orders`). */
export function serviceName(pluginId: string): string {
  return pascal(pluginId);
}

/** The request path of an endpoint: `/<id>.v1.<Service>/<Name>`. */
export function endpointPath(pluginId: string, endpoint: string): string {
  return `/${pluginId}.v1.${serviceName(pluginId)}/${endpoint}`;
}
