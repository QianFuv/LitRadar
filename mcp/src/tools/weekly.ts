import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse } from "../client.js";

function registerWeeklyTools(
  server: McpServer,
  client: PaperScannerClient,
): void {
  server.registerTool(
    "get_weekly_updates",
    {
      description: "Get weekly update summaries across all Paper Scanner databases.",
      inputSchema: z.strictObject({}),
    },
    async () => {
      const result = await client.get("/weekly-updates", {
        auth: true,
      });

      return buildToolResponse(result);
    },
  );
}

export { registerWeeklyTools };
