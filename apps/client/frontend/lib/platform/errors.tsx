/**
 * Typed failures reported by a native host.
 */

/** Error raised when a native host rejects an operation with a stable code. */
export class PlatformBridgeError extends Error {
  /** Stable host error code, such as `insecure_origin` or `server_unreachable`. */
  readonly code: string;

  /**
   * Create a host error with its stable code.
   *
   * @param code - Stable host error code.
   * @param message - Diagnostic message without secrets.
   * @param options - Standard error options carrying the original cause.
   */
  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'PlatformBridgeError';
    this.code = code;
    Object.setPrototypeOf(this, PlatformBridgeError.prototype);
  }
}
