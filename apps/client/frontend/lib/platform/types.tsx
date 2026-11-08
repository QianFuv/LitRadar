/**
 * Host capabilities the client needs from its runtime (browser now, native host later).
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
   */
  saveDownload(blob: Blob, filename: string): void;

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
}
