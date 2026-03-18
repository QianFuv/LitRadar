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
      const result = await client.get("/meta/databases");
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
        db,
      });

      return buildToolResponse(result);
    },
  );
}

export { registerMetaTools };
