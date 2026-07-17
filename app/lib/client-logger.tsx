'use client';

/**
 * Privacy-bounded browser error events written only to the local developer console.
 */

export type ClientErrorSource =
  | 'global_boundary'
  | 'route_boundary'
  | 'unhandled_rejection'
  | 'window_error';

export interface ClientErrorContext {
  readonly digest?: unknown;
  readonly requestId?: unknown;
  readonly routePathname?: string;
}

export interface ClientErrorEvent {
  readonly component: 'browser';
  readonly digest?: string;
  readonly error_kind: string;
  readonly event: 'client.error';
  readonly level: 'error';
  readonly request_id?: string;
  readonly route: string;
  readonly source: ClientErrorSource;
  readonly timestamp: string;
}

const MAX_IDENTIFIER_CHARACTERS = 128;
const MAX_ROUTE_CHARACTERS = 256;
const SAFE_IDENTIFIER_PATTERN = /^[A-Za-z0-9._:-]+$/;
const CONTROL_CHARACTER_PATTERN = /[\u0000-\u001f\u007f]/g;
const ERROR_KINDS_BY_NAME: Readonly<Record<string, string>> = Object.freeze({
  AggregateError: 'aggregate_error',
  ApiError: 'api_error',
  Error: 'error',
  EvalError: 'eval_error',
  RangeError: 'range_error',
  ReferenceError: 'reference_error',
  SyntaxError: 'syntax_error',
  TypeError: 'type_error',
  URIError: 'uri_error',
});
const REPORTED_ERRORS = new WeakSet<object>();

/**
 * Check whether a value can safely be tracked by object identity.
 *
 * @param value - Unknown error or rejection value.
 * @returns Whether the value is a non-null object.
 */
function isObject(value: unknown): value is object {
  return typeof value === 'object' && value !== null;
}

/**
 * Read one explicitly allowed property without enumerating or serializing the source object.
 *
 * @param value - Possible error object.
 * @param property - Allowlisted property name.
 * @returns Property value, or undefined when access fails.
 */
function readAllowedProperty(value: unknown, property: 'digest' | 'name' | 'requestId'): unknown {
  if (!isObject(value)) {
    return undefined;
  }
  try {
    return Reflect.get(value, property);
  } catch {
    return undefined;
  }
}

/**
 * Normalize an optional digest or request identifier to a bounded symbolic token.
 *
 * @param value - Candidate identifier.
 * @returns Safe identifier or undefined.
 */
function normalizeIdentifier(value: unknown): string | undefined {
  if (typeof value !== 'string') {
    return undefined;
  }
  const identifier = value.trim();
  if (
    identifier.length === 0 ||
    identifier.length > MAX_IDENTIFIER_CHARACTERS ||
    !SAFE_IDENTIFIER_PATTERN.test(identifier)
  ) {
    return undefined;
  }
  return identifier;
}

/**
 * Convert an unknown failure into a stable, non-message error classification.
 *
 * @param source - Browser boundary that observed the failure.
 * @param error - Unknown error or rejection value.
 * @returns Stable error kind.
 */
function normalizeErrorKind(source: ClientErrorSource, error: unknown): string {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException) {
    return 'dom_exception';
  }
  if (error instanceof Error) {
    const name = readAllowedProperty(error, 'name');
    if (typeof name === 'string') {
      return ERROR_KINDS_BY_NAME[name] ?? 'unknown_error';
    }
    return 'unknown_error';
  }
  if (source === 'unhandled_rejection') {
    return 'non_error_rejection';
  }
  if (source === 'window_error') {
    return 'script_error';
  }
  return 'unknown_error';
}

/**
 * Reduce a route value to a bounded pathname without query or fragment data.
 *
 * @param value - Candidate route pathname.
 * @returns Safe pathname.
 */
function normalizeRoutePathname(value: string): string {
  const pathname = value.split(/[?#]/, 1)[0].replace(CONTROL_CHARACTER_PATTERN, '');
  if (!pathname.startsWith('/')) {
    return '/';
  }
  return pathname.slice(0, MAX_ROUTE_CHARACTERS) || '/';
}

/**
 * Read the current browser pathname without query or fragment data.
 *
 * @returns Current safe route pathname.
 */
function currentRoutePathname(): string {
  if (typeof window === 'undefined') {
    return '/';
  }
  return normalizeRoutePathname(window.location.pathname);
}

/**
 * Mark an object failure as reported and detect repeated reporting of the same object.
 *
 * @param error - Unknown failure value.
 * @returns Whether the same object was already reported.
 */
function isDuplicateError(error: unknown): boolean {
  if (!isObject(error)) {
    return false;
  }
  if (REPORTED_ERRORS.has(error)) {
    return true;
  }
  REPORTED_ERRORS.add(error);
  return false;
}

/**
 * Build one allowlisted browser error event without serializing the originating failure.
 *
 * @param source - Browser boundary that observed the failure.
 * @param error - Unknown error or rejection value used only for classification.
 * @param context - Optional safe correlation context.
 * @returns Frozen JSON-compatible event object.
 */
export function buildClientErrorEvent(
  source: ClientErrorSource,
  error: unknown,
  context: ClientErrorContext = {},
): ClientErrorEvent {
  const digest = normalizeIdentifier(context.digest ?? readAllowedProperty(error, 'digest'));
  const requestId = normalizeIdentifier(
    context.requestId ?? readAllowedProperty(error, 'requestId'),
  );
  const route = context.routePathname
    ? normalizeRoutePathname(context.routePathname)
    : currentRoutePathname();
  const event: ClientErrorEvent = {
    component: 'browser',
    error_kind: normalizeErrorKind(source, error),
    event: 'client.error',
    level: 'error',
    route,
    source,
    timestamp: new Date().toISOString(),
    ...(digest ? { digest } : {}),
    ...(requestId ? { request_id: requestId } : {}),
  };
  return Object.freeze(event);
}

/**
 * Emit one deduplicated browser error event to the local console only.
 *
 * @param source - Browser boundary that observed the failure.
 * @param error - Unknown error or rejection value.
 * @param context - Optional safe correlation context.
 * @returns Whether a new event was emitted.
 */
export function reportClientError(
  source: ClientErrorSource,
  error: unknown,
  context: ClientErrorContext = {},
): boolean {
  if (isDuplicateError(error)) {
    return false;
  }
  console.error(buildClientErrorEvent(source, error, context));
  return true;
}
