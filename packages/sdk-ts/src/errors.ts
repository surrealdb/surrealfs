/**
 * Error hierarchy for SurrealFS TypeScript SDK.
 */

export class SurrealFsError extends Error {
  constructor(message: string) {
    super(message);
    this.name = this.constructor.name;
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

export class NotFoundError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Not found: ${path}`);
  }
}

export class PermissionDeniedError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Permission denied: ${path}`);
  }
}

export class ConflictError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Conflict on: ${path}`);
  }
}

export class AlreadyExistsError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Already exists: ${path}`);
  }
}

export class DirectoryNotEmptyError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Directory not empty: ${path}`);
  }
}

export class IsADirectoryError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Is a directory: ${path}`);
  }
}

export class NotADirectoryError extends SurrealFsError {
  constructor(public readonly path: string, message?: string) {
    super(message ?? `Not a directory: ${path}`);
  }
}

export function mapSurrealError(err: unknown, path: string = ""): Error {
  if (err instanceof SurrealFsError) return err;
  const msg = err instanceof Error ? err.message : String(err);
  const lower = msg.toLowerCase();

  if (lower.includes("sfs:not_found") || lower.includes("not found")) {
    return new NotFoundError(path, msg);
  }
  if (lower.includes("sfs:permission_denied") || lower.includes("permission denied")) {
    return new PermissionDeniedError(path, msg);
  }
  if (
    lower.includes("sfs:conflict") ||
    lower.includes("sfs:locked") ||
    lower.includes("conflict") ||
    lower.includes("generation mismatch")
  ) {
    return new ConflictError(path, msg);
  }
  if (lower.includes("sfs:already_exists") || lower.includes("already exists")) {
    return new AlreadyExistsError(path, msg);
  }
  if (lower.includes("sfs:not_empty") || lower.includes("directory not empty")) {
    return new DirectoryNotEmptyError(path, msg);
  }
  if (lower.includes("sfs:is_a_directory") || lower.includes("is a directory")) {
    return new IsADirectoryError(path, msg);
  }
  if (lower.includes("sfs:not_a_directory") || lower.includes("not a directory")) {
    return new NotADirectoryError(path, msg);
  }
  return new SurrealFsError(msg);
}
