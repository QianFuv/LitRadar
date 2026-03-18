import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse } from "../client.js";

const databaseSchema = z.string().trim().min(1);

function registerJournalTools(
  server: McpServer,
  client: PaperScannerClient,
): void {
  server.registerTool(
    "list_journals",
    {
      description: "List journals from the selected Paper Scanner database.",
      inputSchema: z.object({
        db: databaseSchema.optional(),
        limit: z.number().int().min(1).max(200).optional(),
        offset: z.number().int().min(0).optional(),
      }),
    },
    async ({ db, limit, offset }) => {
      const result = await client.get("/journals", {
        db,
        query: {
          limit,
          offset,
        },
      });

      return buildToolResponse(result);
    },
  );
}

export { registerJournalTools };
