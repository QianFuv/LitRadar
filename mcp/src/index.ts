#!/usr/bin/env node

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { PaperScannerClient } from "./client.js";
import { registerArticleTools } from "./tools/articles.js";
import { registerFavoriteTools } from "./tools/favorites.js";
import { registerJournalTools } from "./tools/journals.js";
import { registerMetaTools } from "./tools/meta.js";
import { registerWeeklyTools } from "./tools/weekly.js";

async function main(): Promise<void> {
  const server = new McpServer({
    name: "paper-scanner-mcp",
    version: "1.0.0",
  });
  const client = new PaperScannerClient();

  registerArticleTools(server, client);
  registerJournalTools(server, client);
  registerMetaTools(server, client);
  registerWeeklyTools(server, client);
  registerFavoriteTools(server, client);

  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((error: unknown) => {
  const message = error instanceof Error ? error.stack ?? error.message : String(error);
  console.error(message);
  process.exit(1);
});
