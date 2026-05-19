//! Built-in deny rules — destructive commands must always resolve to Deny.

#[path = "../src/hooks/permissions.rs"]
mod permissions;

use permissions::{PermissionEngine, Verdict};

fn deny(cmd: &str) {
    let eng = PermissionEngine::builtin_only();
    let (v, _) = eng.check(cmd);
    assert_eq!(v, Verdict::Deny, "expected Deny for {:?}", cmd);
}

#[test]
fn rm_rf_root_denied() {
    deny("rm -rf /");
    deny("rm -rf /  ");
}

#[test]
fn rm_rf_glob_denied() {
    deny("rm -rf /*");
}

#[test]
fn rm_rf_home_denied() {
    deny("rm -rf $HOME");
    deny("rm -rf ~");
}

#[test]
fn git_reset_hard_denied() {
    deny("git reset --hard");
    deny("git reset --hard HEAD~1");
}

#[test]
fn git_clean_force_denied() {
    deny("git clean -fd");
    deny("git clean -f");
}

#[test]
fn git_checkout_dot_denied() {
    deny("git checkout .");
}

#[test]
fn git_restore_dot_denied() {
    deny("git restore .");
}

#[test]
fn mkfs_denied() {
    deny("mkfs.ext4 /dev/sda1");
    deny("mkfs.xfs /dev/sdb");
}

#[test]
fn dd_to_device_denied() {
    deny("dd if=/dev/zero of=/dev/sda bs=1M");
}

#[test]
fn fork_bomb_denied() {
    deny(":(){ :|:& };:");
}
