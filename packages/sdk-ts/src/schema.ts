/**
 * Schema application utilities for SurrealFS TypeScript SDK.
 */

import type { Surreal } from "surrealdb";
import { FILE_SCHEMA, RECORD_AUTH_SCHEMA } from "./schema_sql.js";

export { FILE_SCHEMA, RECORD_AUTH_SCHEMA };

export function schemaSql(options: { recordAuth?: boolean } = {}): string {
  return options.recordAuth
    ? `${FILE_SCHEMA}\n${RECORD_AUTH_SCHEMA}`
    : FILE_SCHEMA;
}

export async function applySchema(
  db: Surreal,
  options: { recordAuth?: boolean } = {}
): Promise<void> {
  const sql = schemaSql(options);
  await db.query(sql);
}
