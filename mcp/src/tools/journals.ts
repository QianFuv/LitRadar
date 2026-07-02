import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse } from "../client.js";

const databaseSchema = z.string().trim().min(1);
const journalIdSchema = z.string().trim().regex(/^[1-9]\d*$/);
const textSchema = z.string().trim().min(1);

function registerJournalTools(
  server: McpServer,
  client: PaperScannerClient,
): void {
  server.registerTool(
    "list_journals",
    {
      description: "List journals from the selected Paper Scanner database.",
      inputSchema: z.object({
        area: textSchema.optional(),
        available: z.boolean().optional(),
        db: databaseSchema.optional(),
        has_articles: z.boolean().optional(),
        library_id: textSchema.optional(),
        limit: z.number().int().min(1).max(200).optional(),
        offset: z.number().int().min(0).optional(),
        scimago_max: z.number().optional(),
        scimago_min: z.number().optional(),
        sort: textSchema.optional(),
        year: z.number().int().nonnegative().optional(),
      }),
    },
    async (params) => {
      const result = await client.get("/journals", {
        auth: true,
        db: params.db,
        query: {
          area: params.area,
          available: params.available,
          has_articles: params.has_articles,
          library_id: params.library_id,
          limit: params.limit,
          offset: params.offset,
          scimago_max: params.scimago_max,
          scimago_min: params.scimago_min,
          sort: params.sort,
          year: params.year,
        },
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "get_journal",
    {
      description: "Get a single journal by ID.",
      inputSchema: z.object({
        db: databaseSchema.optional(),
        journal_id: journalIdSchema,
      }),
    },
    async ({ db, journal_id }) => {
      const result = await client.get(`/journals/${journal_id}`, {
        auth: true,
        db,
      });

      return buildToolResponse(result);
    },
  );
}

export { registerJournalTools };
