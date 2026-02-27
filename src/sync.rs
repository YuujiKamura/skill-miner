// Auto-sync: commit and push skill drafts to a configured git repository.
// CRITICAL SAFETY: never push to a public repository.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Configuration for syncing drafts to a git repository.
pub struct SyncConfig {
    /// drafts directory path (= git working tree)
    pub drafts_dir: PathBuf,
    /// Remote name (default: "origin")
    pub remote: String,
    /// Branch name (default: "main")
    pub branch: String,
}

/// Result of a sync operation.
pub struct SyncResult {
    pub committed: bool,
    pub pushed: bool,
    pub commit_message: String,
    pub files_changed: usize,
}

/// Run a git command in the given directory and return stdout.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Ensure the drafts directory is a git repository. Init if not.
pub fn ensure_git_repo(dir: &Path) -> Result<()> {
    let git_dir = dir.join(".git");
    if git_dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    git(dir, &["init"])?;
    Ok(())
}

/// Check if a remote is configured for the repository.
fn has_remote(dir: &Path, remote: &str) -> bool {
    git(dir, &["remote", "get-url", remote]).is_ok()
}

/// CRITICAL SAFETY: Check that the remote repository is NOT public.
/// Returns Ok(()) if safe to push, Err if public or cannot verify.
fn verify_not_public(dir: &Path, remote: &str) -> Result<()> {
    let url = git(dir, &["remote", "get-url", remote])
        .with_context(|| format!("cannot get URL for remote '{}'", remote))?;

    // Only check GitHub URLs (the main concern per MEMORY.md)
    if url.contains("github.com") {
        // Extract owner/repo from URL
        let repo_slug = parse_github_slug(&url)
            .with_context(|| format!("cannot parse GitHub slug from URL: {}", url))?;

        // Use gh CLI to check visibility
        let output = Command::new("gh")
            .args(["repo", "view", &repo_slug, "--json", "visibility"])
            .output()
            .context("failed to run 'gh repo view' -- is gh CLI installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "cannot verify repository visibility for '{}': {}",
                repo_slug,
                stderr.trim()
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse {"visibility":"PUBLIC"} or {"visibility":"PRIVATE"}
        if let Some(vis) = stdout
            .split("\"visibility\"")
            .nth(1)
            .and_then(|s| s.split('"').nth(1))
        {
            if vis.eq_ignore_ascii_case("public") {
                bail!(
                    "REFUSING TO PUSH: repository '{}' is PUBLIC. \
                     Pushing skill drafts to a public repo is forbidden. \
                     Change the repo to private first.",
                    repo_slug
                );
            }
        } else {
            bail!(
                "cannot parse visibility from gh output for '{}': {}",
                repo_slug,
                stdout.trim()
            );
        }
    }
    // Non-GitHub remotes: refuse by default (cannot verify visibility)
    bail!(
        "REFUSING TO PUSH: remote URL '{}' is not a GitHub repository. \
         Cannot verify visibility. Only GitHub remotes are supported for auto-sync.",
        url
    )
}

/// Parse "owner/repo" from a GitHub URL.
/// Supports: https://github.com/owner/repo.git, git@github.com:owner/repo.git
fn parse_github_slug(url: &str) -> Option<String> {
    // HTTPS: https://github.com/owner/repo or https://github.com/owner/repo.git
    if let Some(rest) = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
    {
        let slug = rest.trim_end_matches(".git").trim_end_matches('/');
        if slug.contains('/') {
            return Some(slug.to_string());
        }
    }
    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let slug = rest.trim_end_matches(".git").trim_end_matches('/');
        if slug.contains('/') {
            return Some(slug.to_string());
        }
    }
    None
}

/// Sync drafts directory to git: commit changes and optionally push.
/// Push failures are reported but do not cause the overall operation to fail.
pub fn sync_drafts(config: &SyncConfig, new_drafts: usize, updated_drafts: usize) -> SyncResult {
    let mut result = SyncResult {
        committed: false,
        pushed: false,
        commit_message: String::new(),
        files_changed: 0,
    };

    // Ensure git repo exists
    if let Err(e) = ensure_git_repo(&config.drafts_dir) {
        eprintln!("[sync] failed to ensure git repo: {}", e);
        return result;
    }

    // Check for changes
    let status = match git(&config.drafts_dir, &["status", "--porcelain"]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[sync] failed to check git status: {}", e);
            return result;
        }
    };

    if status.is_empty() {
        eprintln!("[sync] no changes to commit");
        return result;
    }

    let files_changed = status.lines().count();
    result.files_changed = files_changed;

    // Stage all changes
    if let Err(e) = git(&config.drafts_dir, &["add", "-A"]) {
        eprintln!("[sync] failed to stage changes: {}", e);
        return result;
    }

    // Build commit message
    let msg = format!(
        "skill-miner: {} drafts ({} new, {} updated)",
        new_drafts + updated_drafts,
        new_drafts,
        updated_drafts
    );
    result.commit_message = msg.clone();

    // Commit
    match git(&config.drafts_dir, &["commit", "-m", &msg]) {
        Ok(_) => {
            result.committed = true;
            eprintln!("[sync] committed: {}", msg);
        }
        Err(e) => {
            eprintln!("[sync] failed to commit: {}", e);
            // Reset staging area to avoid stale staged files on next run
            let _ = git(&config.drafts_dir, &["reset"]);
            return result;
        }
    }

    // Push (only if remote is configured)
    if !has_remote(&config.drafts_dir, &config.remote) {
        eprintln!("[sync] no remote '{}' configured, skipping push", config.remote);
        return result;
    }

    // CRITICAL: verify repo is not public before pushing
    if let Err(e) = verify_not_public(&config.drafts_dir, &config.remote) {
        eprintln!("[sync] {}", e);
        return result;
    }

    match git(
        &config.drafts_dir,
        &["push", &config.remote, &config.branch],
    ) {
        Ok(_) => {
            result.pushed = true;
            eprintln!("[sync] pushed to {}/{}", config.remote, config.branch);
        }
        Err(e) => {
            eprintln!("[sync] push failed (non-fatal): {}", e);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_slug_https() {
        assert_eq!(
            parse_github_slug("https://github.com/user/repo.git"),
            Some("user/repo".to_string())
        );
        assert_eq!(
            parse_github_slug("https://github.com/user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_slug_ssh() {
        assert_eq!(
            parse_github_slug("git@github.com:user/repo.git"),
            Some("user/repo".to_string())
        );
        assert_eq!(
            parse_github_slug("git@github.com:user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_slug_invalid() {
        assert_eq!(parse_github_slug("https://gitlab.com/user/repo"), None);
        assert_eq!(parse_github_slug("not-a-url"), None);
    }

    #[test]
    fn test_ensure_git_repo_creates_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("test-repo");
        ensure_git_repo(&repo_dir).unwrap();
        assert!(repo_dir.join(".git").exists());
    }

    #[test]
    fn test_ensure_git_repo_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("test-repo");
        ensure_git_repo(&repo_dir).unwrap();
        // Second call should succeed without error
        ensure_git_repo(&repo_dir).unwrap();
        assert!(repo_dir.join(".git").exists());
    }

    #[test]
    fn test_sync_drafts_commits_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("sync-test");
        ensure_git_repo(&repo_dir).unwrap();

        // Configure git user for the test repo
        let _ = git(&repo_dir, &["config", "user.email", "test@test.com"]);
        let _ = git(&repo_dir, &["config", "user.name", "Test"]);

        // Write a file
        std::fs::write(repo_dir.join("test-skill.md"), "# Test skill").unwrap();

        let config = SyncConfig {
            drafts_dir: repo_dir.clone(),
            remote: "origin".to_string(),
            branch: "main".to_string(),
        };

        let result = sync_drafts(&config, 1, 0);
        assert!(result.committed);
        assert!(!result.pushed); // no remote configured
        assert_eq!(result.files_changed, 1);
        assert!(result.commit_message.contains("1 new"));
    }

    #[test]
    fn test_sync_no_changes_skips_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("sync-noop");
        ensure_git_repo(&repo_dir).unwrap();

        let config = SyncConfig {
            drafts_dir: repo_dir,
            remote: "origin".to_string(),
            branch: "main".to_string(),
        };

        let result = sync_drafts(&config, 0, 0);
        assert!(!result.committed);
        assert!(!result.pushed);
        assert_eq!(result.files_changed, 0);
    }

    #[test]
    fn test_sync_no_remote_skips_push() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("sync-no-remote");
        ensure_git_repo(&repo_dir).unwrap();

        let _ = git(&repo_dir, &["config", "user.email", "test@test.com"]);
        let _ = git(&repo_dir, &["config", "user.name", "Test"]);

        std::fs::write(repo_dir.join("skill.md"), "content").unwrap();

        let config = SyncConfig {
            drafts_dir: repo_dir,
            remote: "origin".to_string(),
            branch: "main".to_string(),
        };

        let result = sync_drafts(&config, 1, 0);
        assert!(result.committed);
        assert!(!result.pushed);
    }
}
