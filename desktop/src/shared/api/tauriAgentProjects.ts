import { invokeTauri } from "@/shared/api/tauri";

/**
 * Check the folder named in an agent's project request: the picker's own rules
 * plus a narrower one for requests — inside the owner's home, not the home
 * itself, not `~/Library`, not hidden. Resolves to the canonical path.
 */
export async function checkAgentProjectFolder(path: string): Promise<string> {
  return invokeTauri<string>("check_agent_project_folder", { path });
}
