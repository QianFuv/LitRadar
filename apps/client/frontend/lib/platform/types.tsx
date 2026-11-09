/**
 * Host capabilities the client needs from its runtime: the browser or a native Wails host.
 */

/** Runtime-specific transport and host integration used by the shared client code. */
export interface ClientPlatform {
  /**
   * Send one HTTP request to the LitRadar server.
   *
   * @param url - Absolute request URL.
   * @param init - Request options, including the cancellation signal.
   * @returns Raw server response.
   */
  send(url: string, init: RequestInit): Promise<Response>;

  /**
   * Hand a downloaded file to the user.
   *
   * @param blob - File contents.
   * @param filename - Suggested file name.
   * @returns Whether the file was saved; false when the user cancelled.
   */
  saveDownload(blob: Blob, filename: string): Promise<boolean>;

  /**
   * Open a URL outside the application.
   *
   * @param url - Absolute external URL.
   */
  openExternal(url: string): void;

  /**
   * Resolve the origin of the LitRadar server that receives API requests.
   *
   * @returns Absolute server origin without a trailing slash.
   */
  serverOrigin(): string;

  /**
   * Install document-wide host integration before the application renders.
   */
  prepareDocument(): void;

  /** Whether sign-in may navigate the page to an identity provider and back. */
  supportsSingleSignOn: boolean;

  /** Host-owned credential operations; null when a browser cookie carries the session. */
  nativeSession: NativeSession | null;

  /** User-selected server management; null when the page origin is the server. */
  serverConnection: ServerConnection | null;
}

/**
 * Credential operations of a native host that keeps the bearer token out of JavaScript.
 *
 * Every method resolves with the server response so the shared client can parse
 * errors, and rejects like `fetch` on transport failures and cancellation.
 */
export interface NativeSession {
  /**
   * Sign in with a password and let the host store the issued token.
   *
   * @param username - Username or account email.
   * @param password - Password.
   * @param signal - Cancellation signal.
   * @returns Login response without the token.
   */
  login(username: string, password: string, signal: AbortSignal | null): Promise<Response>;

  /**
   * Validate a pasted personal access token and let the host store it when accepted.
   *
   * @param token - Personal access token.
   * @param signal - Cancellation signal.
   * @returns Current-user response for the token.
   */
  importAccessToken(token: string, signal: AbortSignal | null): Promise<Response>;

  /**
   * Revoke the stored token on the server; the host deletes it locally regardless.
   *
   * @param signal - Cancellation signal.
   * @returns Logout response.
   */
  logout(signal: AbortSignal | null): Promise<Response>;

  /**
   * Delete the stored token without contacting the server.
   *
   * @returns Promise settled after deletion.
   */
  discardCredential(): Promise<void>;
}

/** Server selection kept by a native host. */
export interface ServerConnection {
  /**
   * Read the stored server origin and make it the active request origin.
   *
   * @returns Stored origin, or null before setup.
   */
  load(): Promise<string | null>;

  /**
   * Probe and store a user-entered server address.
   *
   * @param address - Server address entered by the user.
   * @returns Canonical stored origin.
   */
  connect(address: string): Promise<string>;

  /**
   * Revoke and delete the token of the active server and forget the server.
   *
   * @returns Promise settled after the host forgot the server.
   */
  disconnect(): Promise<void>;
}
