import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse, toArray } from "../client.js";

const articleIdSchema = z.string().trim().regex(/^[1-9]\d*$/);
const journalIdSchema = z.string().trim().regex(/^[1-9]\d*$/);
const dateSchema = z.string().trim().min(1);
const databaseSchema = z.string().trim().min(1);
const articleListSchema = z.object({
  area: z.union([z.string().trim().min(1), z.array(z.string().trim().min(1)).min(1)]).optional(),
  date_from: dateSchema.optional(),
  date_to: dateSchema.optional(),
  db: databaseSchema.optional(),
  journal_id: z.union([journalIdSchema, z.array(journalIdSchema).min(1)]).optional(),
  limit: z.number().int().min(1).max(200).optional(),
  open_access: z.boolean().optional(),
  q: z.string().trim().min(1).optional(),
  year: z.number().int().nonnegative().optional(),
});

function registerArticleTools(
  server: McpServer,
  client: PaperScannerClient,
): void {
  server.registerTool(
    "search_articles",
    {
      description: "Search articles in the Paper Scanner index.",
      inputSchema: articleListSchema,
    },
    async (params) => {
      const result = await client.get("/articles", {
        auth: true,
        db: params.db,
        query: {
          area: toArray(params.area),
          date_from: params.date_from,
          date_to: params.date_to,
          journal_id: toArray(params.journal_id),
          limit: params.limit,
          open_access: params.open_access,
          q: params.q,
          year: params.year,
        },
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "get_article",
    {
      description: "Get a single article by ID.",
      inputSchema: z.object({
        article_id: articleIdSchema,
        db: databaseSchema.optional(),
      }),
    },
    async ({ article_id, db }) => {
      const result = await client.get(`/articles/${article_id}`, {
        auth: true,
        db,
      });

      return buildToolResponse(result);
    },
  );
}

export { registerArticleTools };
