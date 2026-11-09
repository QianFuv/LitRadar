/**
 * Typed failures reported by a native host.
 */

/** Error raised when a native host rejects an operation with a stable code. */
export class PlatformBridgeError extends Error {
  /** Stable host error code, such as `insecure_origin` or `server_unreachable`. */
  readonly code: string;

  /** Bounded detail of an `oidc_error`, such as the server's `oidc_error` value; else null. */
  readonly reason: string | null;

  /**
   * Create a host error with its stable code.
   *
   * @param code - Stable host error code.
   * @param message - Diagnostic message without secrets.
   * @param options - Standard error options carrying the original cause.
   * @param reason - Optional bounded failure detail.
   */
  constructor(code: string, message: string, options?: ErrorOptions, reason: string | null = null) {
    super(message, options);
    this.name = 'PlatformBridgeError';
    this.code = code;
    this.reason = reason;
    Object.setPrototypeOf(this, PlatformBridgeError.prototype);
  }
}
