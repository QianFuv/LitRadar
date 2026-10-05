/** Remove every task-owned resource and retain failures instead of reporting a false pass. */
export function cleanup(command, containers, network) {
  const results = [];
  for (const [kind, name] of [
    ...[...containers].map((name) => ["container", name]),
    ...(network ? [["network", network]] : []),
  ]) {
    try {
      const removed = command(
        "docker",
        kind === "container"
          ? ["rm", "--force", name]
          : ["network", "rm", name],
        true,
      );
      let absent;
      if (removed.status !== 0) {
        absent = command(
          "docker",
          kind === "container"
            ? ["ps", "--all", "--quiet", "--filter", `name=^/${name}$`]
            : ["network", "ls", "--quiet", "--filter", `name=^${name}$`],
          true,
        );
      }
      results.push({
        kind,
        name,
        removed,
        absent,
        success:
          removed.status === 0 ||
          (absent.status === 0 && absent.stdout.trim() === ""),
      });
    } catch (error) {
      results.push({ kind, name, success: false, error: String(error) });
    }
  }
  return results;
}
