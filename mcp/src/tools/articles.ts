import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse, toArray } from "../client.js";

const articleIdSchema = z.string().trim().regex(/^[1-9]\d*$/);
const journalIdSchema = z.string().trim().regex(/^[1-9]\d*$/);
const dateSchema = z.string().trim().min(1);
const databaseSchema = z.string().trim().min(1);
const textSchema = z.string().trim().min(1);
const articleListSchema = z.object({
  area: z.union([textSchema, z.array(textSchema).min(1)]).optional(),
  cursor: textSchema.optional(),
  date_from: dateSchema.optional(),
  date_to: dateSchema.optional(),
  db: databaseSchema.optional(),
  doi: textSchema.optional(),
  include_total: z.boolean().optional(),
  in_press: z.boolean().optional(),
  issue_id: z.number().int().nonnegative().optional(),
  journal_id: z.union([journalIdSchema, z.array(journalIdSchema).min(1)]).optional(),
  limit: z.number().int().min(1).max(200).optional(),
  open_access: z.boolean().optional(),
  offset: z.number().int().nonnegative().optional(),
  pmid: textSchema.optional(),
  q: textSchema.optional(),
  sort: textSchema.optional(),
  suppressed: z.boolean().optional(),
  within_library_holdings: z.boolean().optional(),
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
          cursor: params.cursor,
          date_from: params.date_from,
          date_to: params.date_to,
          doi: params.doi,
          include_total: params.include_total,
          in_press: params.in_press,
          issue_id: params.issue_id,
          journal_id: toArray(params.journal_id),
          limit: params.limit,
          open_access: params.open_access,
          offset: params.offset,
          pmid: params.pmid,
          q: params.q,
          sort: params.sort,
          suppressed: params.suppressed,
          within_library_holdings: params.within_library_holdings,
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
