type Primitive = boolean | number | string;

type QueryValue = Primitive | Primitive[] | null | undefined;

type QueryParams = Record<string, QueryValue>;

type RequestOptions = {
  auth?: boolean;
  body?: unknown;
  db?: string;
  query?: QueryParams;
};

class PaperScannerClient {
  private readonly apiToken: string | null;
  private readonly baseUrl: string;
  private readonly defaultDb: string | null;

  constructor() {
    this.baseUrl = this.normalizeBaseUrl(
      process.env.PAPER_SCANNER_API_URL ?? "http://localhost:8000",
    );
    this.apiToken = this.readOptionalEnv("PAPER_SCANNER_API_TOKEN");
    this.defaultDb = this.readOptionalEnv("PAPER_SCANNER_DB");
  }

  getDefaultDb(): string | null {
    return this.defaultDb;
  }

  async delete<T>(path: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>("DELETE", path, options);
  }

  async get<T>(path: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>("GET", path, options);
  }

  async post<T>(path: string, options: RequestOptions = {}): Promise<T> {
    return this.request<T>("POST", path, options);
  }

  private appendQueryParams(
    searchParams: URLSearchParams,
    query: QueryParams | undefined,
  ): void {
    if (!query) {
      return;
    }

    for (const [key, value] of Object.entries(query)) {
      if (value === undefined || value === null) {
        continue;
      }

      if (Array.isArray(value)) {
        for (const item of value) {
          searchParams.append(key, String(item));
        }
        continue;
      }

      searchParams.set(key, String(value));
    }
  }

  private buildUrl(path: string, options: RequestOptions): URL {
    const normalizedPath = path.startsWith("/") ? path : `/${path}`;
    const url = new URL(`/api${normalizedPath}`, this.baseUrl);
    const db = options.db ?? this.defaultDb;

    if (db) {
      url.searchParams.set("db", db);
    }

    this.appendQueryParams(url.searchParams, options.query);
    return url;
  }

  private normalizeBaseUrl(value: string): string {
    return value.endsWith("/") ? value.slice(0, -1) : value;
  }

  private readOptionalEnv(name: string): string | null {
    const value = process.env[name]?.trim();
    return value ? value : null;
  }

  private async request<T>(
    method: string,
    path: string,
    options: RequestOptions,
  ): Promise<T> {
    if (options.auth && !this.apiToken) {
      throw new Error(
        "PAPER_SCANNER_API_TOKEN is required for authenticated tools",
      );
    }

    const headers = new Headers();
    if (options.auth && this.apiToken) {
      headers.set("Authorization", `Bearer ${this.apiToken}`);
    }

    const init: RequestInit = {
      method,
      headers,
    };

    if (options.body !== undefined) {
      headers.set("Content-Type", "application/json");
      init.body = JSON.stringify(options.body);
    }

    const response = await fetch(this.buildUrl(path, options), init);
    const text = await response.text();

    if (!response.ok) {
      const suffix = text ? `: ${text}` : "";
      throw new Error(
        `Paper Scanner API request failed with ${response.status} ${response.statusText}${suffix}`,
      );
    }

    if (!text) {
      return undefined as T;
    }

    return JSON.parse(text) as T;
  }
}

function buildToolResponse(payload: unknown) {
  const text =
    typeof payload === "string" ? payload : JSON.stringify(payload, null, 2);

  return {
    content: [
      {
        type: "text" as const,
        text,
      },
    ],
  };
}

function toArray<T>(value: T | T[] | undefined): T[] | undefined {
  if (value === undefined) {
    return undefined;
  }

  return Array.isArray(value) ? value : [value];
}

export { PaperScannerClient, buildToolResponse, toArray };
