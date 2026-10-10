// Pi extension: metadata_emit.
//
// WHAT THIS IS
//
// Pi loads this extension in a session that scaffolds or edits a module. After an edit
// lands, it reads the module's `plugin.toml` and writes the module's `metadata.ttl`
// beside it, so the sandbox has a code-metadata document before `mm-cli codex scan`
// runs over the whole tree.
//
// WHAT IT IS NOT
//
// It is not the authority for module metadata. `mm-cli codex scan` is: it derives
// `modules/registry.json` and `codex.lock` from the tree, and it is what the gate
// verifies. This file only gives the session a fast, local view of the one module it
// just touched. `metadata.ttl` is generated and git-ignored (see `.gitignore`), so
// nothing here can disagree with a committed artifact.
//
// It also cannot be run by this repository: it is TypeScript executed by the installed
// `pi` binary, not by `cargo`. Nothing in the Rust build imports, typechecks or tests
// it, and it carries no test claim. Treat it as configuration for an external process,
// the same way the RPC client treats the `pi` binary itself.

/** The shape `plugin.toml` gives us. Only the fields this extension reads. */
export interface PluginManifest {
  plugin: { name: string; uri: string; version: string };
  metadata: { category: string; owned_by_phase: number; capability: string };
  tbox?: { functions?: Record<string, { source: string }> };
}

/** Everything the emitter needs to write one module's metadata. */
export interface MetadataRequest {
  /** Absolute path of the module directory inside the sandbox worktree. */
  moduleDir: string;
  /** The manifest, already parsed by the caller (a TOML parse is Pi's job, not this file's). */
  manifest: PluginManifest;
  /** The sandbox root; the emitter refuses to write outside it. */
  sandboxRoot: string;
}

/** What the emitter did, so the session can report it. */
export interface MetadataResult {
  /** The path written, or null when nothing was written. */
  path: string | null;
  /** The `mmc:` terms emitted, in a stable order. */
  terms: string[];
  /** Why nothing was written, when it was not. */
  skipReason?: string;
}

/** Escape a Turtle literal. */
function turtle(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n");
}

/** True when `path` is inside `root`, so an edit cannot escape its sandbox. */
export function isInside(root: string, path: string): boolean {
  const normalizedRoot = root.endsWith("/") ? root : `${root}/`;
  return path === root || path.startsWith(normalizedRoot);
}

/**
 * The Turtle document for one module.
 *
 * The terms are the ones `ontology/code.ttl` declares: a `mmc:Module` with its
 * `mmc:uri`, `mmc:version`, `mmc:ownedByPhase`, `mmc:path`, and its capability linked to
 * the module (`mmc:implementsCapability`). Nothing is invented here; a term that is not
 * in the ontology would be a validation failure rather than a new fact.
 */
export function renderMetadata(modulePath: string, manifest: PluginManifest): string {
  const { plugin, metadata } = manifest;
  const lines = [
    "@prefix mmc: <https://metamind.dev/code#> .",
    "@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .",
    "",
    `<${plugin.uri}> a mmc:Module ;`,
    `    mmc:name "${turtle(plugin.name)}" ;`,
    `    mmc:uri "${turtle(plugin.uri)}" ;`,
    `    mmc:version "${turtle(plugin.version)}" ;`,
    `    mmc:path "${turtle(modulePath)}" ;`,
    `    mmc:ownedByPhase "${metadata.owned_by_phase}"^^xsd:integer ;`,
    `    mmc:implementsCapability <${plugin.uri}#capability> .`,
    "",
    `<${plugin.uri}#capability> a mmc:Capability ;`,
    `    mmc:name "${turtle(metadata.capability)}" .`,
    "",
  ];
  return lines.join("\n");
}

/**
 * Write `metadata.ttl` for one module.
 *
 * Refuses rather than guesses: a module outside the sandbox is not written, and a
 * manifest with no capability is not written, because the artifact's whole purpose is to
 * be a *checkable* summary of the module and a half-written one is not that.
 */
export async function emitMetadata(
  request: MetadataRequest,
  writeFile: (path: string, contents: string) => Promise<void>,
): Promise<MetadataResult> {
  const { moduleDir, manifest, sandboxRoot } = request;
  const target = `${moduleDir.replace(/\/$/, "")}/metadata.ttl`;

  if (!isInside(sandboxRoot, moduleDir)) {
    return { path: null, terms: [], skipReason: "module is outside the sandbox" };
  }
  if (!manifest.metadata?.capability) {
    return { path: null, terms: [], skipReason: "manifest declares no capability" };
  }

  const terms = ["mmc:Module", "mmc:name", "mmc:uri", "mmc:version", "mmc:path",
                 "mmc:ownedByPhase", "mmc:implementsCapability", "mmc:Capability"];
  await writeFile(target, renderMetadata(moduleDir, manifest));
  return { path: target, terms };
}

/** The hook Pi calls after an edit lands. */
export async function afterEdit(
  context: { sandboxRoot: string; writeFile: (path: string, contents: string) => Promise<void> },
  editedModule: { dir: string; manifest: PluginManifest } | null,
): Promise<MetadataResult> {
  if (!editedModule) {
    return { path: null, terms: [], skipReason: "the edit did not land in a module" };
  }
  return emitMetadata(
    {
      moduleDir: editedModule.dir,
      manifest: editedModule.manifest,
      sandboxRoot: context.sandboxRoot,
    },
    context.writeFile,
  );
}
