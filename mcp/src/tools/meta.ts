import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse } from "../client.js";

const databaseInputSchema = z.object({
  db: z.string().trim().min(1).optional(),
});

function registerMetaTools(
  server: McpServer,
  client: PaperScannerClient,
): void {
  server.registerTool(
    "list_databases",
    {
      description: "List available Paper Scanner SQLite databases.",
      inputSchema: z.object({}),
    },
    async () => {
      const result = await client.get("/meta/databases", {
        auth: true,
      });
      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "list_areas",
    {
      description: "List research areas for the selected Paper Scanner database.",
      inputSchema: databaseInputSchema,
    },
    async ({ db }) => {
      const result = await client.get("/meta/areas", {
        auth: true,
        db,
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "list_years",
    {
      description: "List publication years for the selected Paper Scanner database.",
      inputSchema: databaseInputSchema,
    },
    async ({ db }) => {
      const result = await client.get("/years", {
        auth: true,
        db,
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "list_journal_options",
    {
      description: "List journal filter options for the selected Paper Scanner database.",
      inputSchema: databaseInputSchema,
    },
    async ({ db }) => {
      const result = await client.get("/meta/journals", {
        auth: true,
        db,
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "list_sources",
    {
      description: "List metadata source values for the selected Paper Scanner database.",
      inputSchema: databaseInputSchema,
    },
    async ({ db }) => {
      const result = await client.get("/meta/sources", {
        auth: true,
        db,
      });

      return buildToolResponse(result);
    },
  );
}

export { registerMetaTools };
