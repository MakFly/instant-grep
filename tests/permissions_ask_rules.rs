//! Built-in ask rules — sensitive but non-destructive commands must resolve
//! to Ask (the hook layer surfaces them for user confirmation).

#[path = "../src/hooks/permissions.rs"]
mod permissions;

use permissions::{PermissionEngine, Verdict};

fn ask(cmd: &str) {
    let eng = PermissionEngine::builtin_only();
    let (v, _) = eng.check(cmd);
    assert_eq!(v, Verdict::Ask, "expected Ask for {:?}", cmd);
}

#[test]
fn git_push_force_asks() {
    ask("git push --force origin main");
    ask("git push --force-with-lease origin develop");
    ask("git push -f origin main");
}

#[test]
fn npm_publish_asks() {
    ask("npm publish");
    ask("npm publish --tag beta");
}

#[test]
fn cargo_publish_asks() {
    ask("cargo publish");
    ask("cargo publish --token xyz");
}

#[test]
fn docker_system_prune_asks() {
    ask("docker system prune -a -f");
}

#[test]
fn kubectl_delete_namespace_asks() {
    ask("kubectl delete namespace prod");
    ask("kubectl delete ns prod");
}

#[test]
fn terraform_destroy_asks() {
    ask("terraform destroy");
    ask("terraform destroy -auto-approve");
}

#[test]
fn aws_iam_delete_asks() {
    ask("aws iam delete-user --user-name foo");
}
