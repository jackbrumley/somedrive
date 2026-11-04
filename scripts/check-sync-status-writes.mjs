import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

const repoRoot = process.cwd();
const appRoot = path.join(repoRoot, "src-tauri", "src", "app");

const allowedFiles = new Set([
  path.join(appRoot, "sync_runtime.rs"),
  path.join(appRoot, "sync_engine", "lifecycle_writer.rs"),
  path.join(appRoot, "sync_engine", "runtime_and_models.rs"),
  path.join(appRoot, "commands", "accounts", "profile_commands.rs"),
]);

const forbiddenPattern = /sync_runtime::(?:set_|clear_|record_|start_transfer|update_transfer_progress|finish_transfer|remove_account)/g;

const lifecycleAllowedFiles = new Set([
  path.join(appRoot, "sync_engine", "lifecycle_writer.rs"),
  path.join(appRoot, "sync_engine", "job_queue_lifecycle_store.rs"),
  path.join(appRoot, "sync_engine", "job_queue_issue_throttle_store.rs"),
]);

const forbiddenLifecycleWritePattern = /(persist_sync_lifecycle_(?:phase|activity|remote_scan_complete)\(|persist_sync_issue\(|clear_persisted_sync_issue\()/g;

const operationalLifecycleAllowedFiles = new Set([
  path.join(appRoot, "sync_engine", "lifecycle_writer.rs"),
  path.join(appRoot, "sync_engine", "job_queue_lifecycle_store.rs"),
]);

const forbiddenOperationalLifecycleWritePattern = /persist_sync_lifecycle_operational_state\(/g;

const plannerAllowedFiles = new Set([
  path.join(appRoot, "sync_engine", "cycle_orchestrator.rs"),
  path.join(appRoot, "sync_engine", "remote_changes.rs"),
  path.join(appRoot, "sync_engine", "remote_pipeline_loop.rs"),
  path.join(appRoot, "sync_engine", "planner_transitions.rs"),
  path.join(appRoot, "sync_engine", "job_materializer.rs"),
  // Test-only modules (#[cfg(test)] in sync_engine.rs) exercise the planner.
  path.join(appRoot, "sync_engine", "planner_tests.rs"),
  path.join(appRoot, "sync_engine", "cloud_state_tests.rs"),
  path.join(appRoot, "sync_engine", "test_support.rs"),
]);

const forbiddenPlannerTransitionPattern = /(recompute_sync_file_actions\(|materialize_planner_actions\(|materialize_planner_download_jobs\()/g;

const plannerWriteAllowedFiles = new Set([
  path.join(appRoot, "sync_engine", "planner_transitions.rs"),
]);

const forbiddenPlannerWritePattern = /SET\s+desired_action\s*=|desired_action\s*=\s*CASE/g;

async function collectRustFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectRustFiles(absolute)));
      continue;
    }
    if (entry.isFile() && entry.name.endsWith(".rs")) {
      files.push(absolute);
    }
  }
  return files;
}

async function main() {
  const rustFiles = await collectRustFiles(appRoot);
  const violations = [];

  for (const rustFile of rustFiles) {
    if (allowedFiles.has(rustFile)) {
      continue;
    }
    const content = await readFile(rustFile, "utf8");
    const match = content.match(forbiddenPattern);
    if (match && match.length > 0) {
      violations.push(path.relative(repoRoot, rustFile));
    }
  }

  const lifecycleViolations = [];
  for (const rustFile of rustFiles) {
    if (lifecycleAllowedFiles.has(rustFile) || rustFile.endsWith("_tests.rs")) {
      continue;
    }
    const content = await readFile(rustFile, "utf8");
    const match = content.match(forbiddenLifecycleWritePattern);
    if (match && match.length > 0) {
      lifecycleViolations.push(path.relative(repoRoot, rustFile));
    }
  }

  if (violations.length > 0) {
    console.error("Forbidden sync_runtime write calls found outside central writer files:");
    for (const violation of violations) {
      console.error(` - ${violation}`);
    }
    process.exit(1);
  }

  if (lifecycleViolations.length > 0) {
    console.error("Forbidden lifecycle direct-write calls found outside lifecycle writer/store:");
    for (const violation of lifecycleViolations) {
      console.error(` - ${violation}`);
    }
    process.exit(1);
  }

  const operationalLifecycleViolations = [];
  for (const rustFile of rustFiles) {
    if (operationalLifecycleAllowedFiles.has(rustFile) || rustFile.endsWith("_tests.rs")) {
      continue;
    }
    const content = await readFile(rustFile, "utf8");
    const match = content.match(forbiddenOperationalLifecycleWritePattern);
    if (match && match.length > 0) {
      operationalLifecycleViolations.push(path.relative(repoRoot, rustFile));
    }
  }

  if (operationalLifecycleViolations.length > 0) {
    console.error("Forbidden lifecycle operational-state writes found outside owner files:");
    for (const violation of operationalLifecycleViolations) {
      console.error(` - ${violation}`);
    }
    process.exit(1);
  }

  const plannerViolations = [];
  for (const rustFile of rustFiles) {
    if (plannerAllowedFiles.has(rustFile) || rustFile.endsWith("_tests.rs")) {
      continue;
    }
    const content = await readFile(rustFile, "utf8");
    const match = content.match(forbiddenPlannerTransitionPattern);
    if (match && match.length > 0) {
      plannerViolations.push(path.relative(repoRoot, rustFile));
    }
  }

  if (plannerViolations.length > 0) {
    console.error("Forbidden planner transition/materialization calls found outside orchestrator/planner owners:");
    for (const violation of plannerViolations) {
      console.error(` - ${violation}`);
    }
    process.exit(1);
  }

  const plannerWriteViolations = [];
  for (const rustFile of rustFiles) {
    if (plannerWriteAllowedFiles.has(rustFile) || rustFile.endsWith("_tests.rs")) {
      continue;
    }
    const content = await readFile(rustFile, "utf8");
    const match = content.match(forbiddenPlannerWritePattern);
    if (match && match.length > 0) {
      plannerWriteViolations.push(path.relative(repoRoot, rustFile));
    }
  }

  if (plannerWriteViolations.length > 0) {
    console.error("Forbidden planner desired_action writes found outside planner transitions owner:");
    for (const violation of plannerWriteViolations) {
      console.error(` - ${violation}`);
    }
    process.exit(1);
  }

  console.log("Sync status writer guard passed.");
}

main().catch((error) => {
  console.error(`Failed sync status writer guard: ${String(error)}`);
  process.exit(1);
});
