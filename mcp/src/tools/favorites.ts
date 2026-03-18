import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as z from "zod/v4";
import { PaperScannerClient, buildToolResponse } from "../client.js";

const articleIdSchema = z.number().int().positive();
const folderIdSchema = z.number().int().positive();
const databaseNameSchema = z.string().trim().min(1);

function registerFavoriteTools(
  server: McpServer,
  client: PaperScannerClient,
): void {
  server.registerTool(
    "list_folders",
    {
      description: "List favorite folders for the authenticated Paper Scanner user.",
      inputSchema: z.object({}),
    },
    async () => {
      const result = await client.get("/favorites/folders", {
        auth: true,
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "add_favorite",
    {
      description: "Add an article to a favorite folder for the authenticated user.",
      inputSchema: z.object({
        article_id: articleIdSchema,
        db_name: databaseNameSchema.optional(),
        folder_id: folderIdSchema,
      }),
    },
    async ({ article_id, db_name, folder_id }) => {
      const result = await client.post(`/favorites/folders/${folder_id}/articles`, {
        auth: true,
        body: {
          article_id,
          db_name: db_name ?? client.getDefaultDb() ?? "",
        },
      });

      return buildToolResponse(result);
    },
  );

  server.registerTool(
    "remove_favorite",
    {
      description: "Remove an article from a favorite folder for the authenticated user.",
      inputSchema: z.object({
        article_id: articleIdSchema,
        db_name: databaseNameSchema.optional(),
        folder_id: folderIdSchema,
      }),
    },
    async ({ article_id, db_name, folder_id }) => {
      const result = await client.delete(
        `/favorites/folders/${folder_id}/articles/${article_id}`,
        {
          auth: true,
          query: {
            db_name: db_name ?? client.getDefaultDb() ?? "",
          },
        },
      );

      return buildToolResponse(result);
    },
  );
}

export { registerFavoriteTools };
