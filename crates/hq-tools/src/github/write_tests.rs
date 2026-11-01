use super::*;
use crate::github::tarball::{BaseFile, base_record_path};
use sha2::{Digest, Sha256};
use std::fs;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REPO: &str = "o/r";
const BASE_SHA: &str = "1111111111111111111111111111111111111111";
const TREE_SHA: &str = "2222222222222222222222222222222222222222";
const NEW_TREE: &str = "3333333333333333333333333333333333333333";
const NEW_COMMIT: &str = "4444444444444444444444444444444444444444";
const OTHER_COMMIT: &str = "5555555555555555555555555555555555555555";
const BLOB_SHA: &str = "6666666666666666666666666666666666666666";
const PR_URL: &str = "https://github.com/o/r/pull/7";
const TOKEN: &str = "test-write-token";

fn base_file(path: &str, content: &[u8]) -> BaseFile {
    BaseFile {
        path: path.into(),
        mode: "100644".into(),
        sha256: hex::encode(Sha256::digest(content)),
        size: content.len() as u64,
    }
}

/// A checkout of o/r fetched at BASE_SHA with `keep.txt`, `edit.txt` and `gone.txt`.
fn checkout() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join(REPO);
    fs::create_dir_all(&dest).unwrap();
    for name in ["keep.txt", "edit.txt"] {
        fs::write(dest.join(name), b"same").unwrap();
    }
    let base = Base {
        repo: REPO.into(),
        commit: BASE_SHA.into(),
        files: vec![
            base_file("edit.txt", b"same"),
            base_file("gone.txt", b"old"),
            base_file("keep.txt", b"same"),
        ],
    };
    fs::write(base_record_path(&dest), serde_json::to_vec(&base).unwrap()).unwrap();
    (root, dest)
}

fn base_of(dest: &Path) -> Base {
    load_base(dest).unwrap()
}

fn args(branch: &str, message: &str) -> Value {
    json!({"repo": REPO, "branch": branch, "message": message})
}

fn allowed() -> Vec<String> {
    vec![REPO.to_string()]
}

#[test]
fn only_allowlisted_repositories_and_plain_branch_names_pass() {
    let err = Request::parse(&args("x", "m"), &[]).err().unwrap();
    assert!(err.to_string().contains("github_write.repos"));
    for bad in ["../x", "-x", "a//b", "x.lock", "a..b", "a b", "/x", "x/"] {
        assert!(
            Request::parse(&args(bad, "m"), &allowed()).is_err(),
            "{bad}"
        );
    }
    let ok = Request::parse(&args("hq/fix-log", "Fix the log\n\nDetails"), &allowed()).unwrap();
    assert_eq!(ok.branch, "hq/fix-log", "a given hq/ prefix is not doubled");
    assert_eq!(ok.title, "Fix the log");
    assert!(ok.open_pr);
}

#[test]
fn credentials_in_the_published_text_are_refused() {
    let key = format!("sk-ant-api03-{}", "a1".repeat(20));
    let in_message = Request::parse(&args("x", &format!("use {key}")), &allowed());
    assert!(in_message.err().unwrap().to_string().contains("`message`"));
    let mut a = args("x", "m");
    a["body"] = json!(format!("token {key}"));
    assert!(
        Request::parse(&a, &allowed())
            .err()
            .unwrap()
            .to_string()
            .contains("`body`")
    );
}

#[test]
fn changes_find_new_changed_and_deleted_files_with_their_modes() {
    let (_root, dest) = checkout();
    fs::write(dest.join("edit.txt"), b"different").unwrap();
    fs::create_dir_all(dest.join("src")).unwrap();
    fs::write(dest.join("src/new.rs"), b"fn main() {}").unwrap();
    let found = changes(&dest, &base_of(&dest)).unwrap();
    let summary: Vec<(&str, bool)> = found
        .iter()
        .map(|c| (c.path.as_str(), c.content.is_some()))
        .collect();
    assert_eq!(
        summary,
        [
            ("edit.txt", true),
            ("src/new.rs", true),
            ("gone.txt", false)
        ]
    );
    assert_eq!(found[2].mode, "100644");
}

#[cfg(unix)]
#[test]
fn a_link_in_the_checkout_refuses_the_push_and_is_never_read() {
    let (_root, dest) = checkout();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("config.yaml"), b"secret").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("config.yaml"),
        dest.join("innocent.txt"),
    )
    .unwrap();
    let err = changes(&dest, &base_of(&dest)).unwrap_err();
    assert!(err.to_string().contains("link"), "{err}");
}

#[test]
fn git_metadata_and_credential_files_refuse_the_push() {
    let (_root, dest) = checkout();
    fs::create_dir_all(dest.join(".git/hooks")).unwrap();
    assert!(
        changes(&dest, &base_of(&dest))
            .unwrap_err()
            .to_string()
            .contains("git metadata")
    );
    fs::remove_dir_all(dest.join(".git")).unwrap();

    let pem = "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----\n";
    fs::write(dest.join("leak.txt"), pem).unwrap();
    let err = changes(&dest, &base_of(&dest)).unwrap_err().to_string();
    assert!(
        err.contains("leak.txt") && err.contains("credential"),
        "{err}"
    );
    assert!(
        !err.contains("BEGIN RSA"),
        "the refusal never repeats the value"
    );
}

#[cfg(unix)]
#[test]
fn a_mode_change_alone_counts_as_a_change() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, dest) = checkout();
    fs::set_permissions(dest.join("keep.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    let found = changes(&dest, &base_of(&dest)).unwrap();
    assert!(
        found
            .iter()
            .any(|c| c.path == "keep.txt" && c.mode == "100755")
    );
}

async fn mount(server: &MockServer, verb: &str, at: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(format!("/repos/{REPO}/{at}")))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// GitHub for one push of a new branch and a new pull request.
async fn github(branch_head: Option<&str>) -> MockServer {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        &format!("git/commits/{BASE_SHA}"),
        200,
        json!({"tree": {"sha": TREE_SHA}}),
    )
    .await;
    match branch_head {
        Some(sha) => {
            mount(
                &server,
                "GET",
                "git/ref/heads/hq/fix",
                200,
                json!({"object": {"sha": sha}}),
            )
            .await
        }
        None => {
            mount(
                &server,
                "GET",
                "git/ref/heads/hq/fix",
                404,
                json!({"message": "Not Found"}),
            )
            .await
        }
    }
    mount(&server, "POST", "git/blobs", 201, json!({"sha": BLOB_SHA})).await;
    mount(&server, "POST", "git/trees", 201, json!({"sha": NEW_TREE})).await;
    mount(
        &server,
        "POST",
        "git/commits",
        201,
        json!({"sha": NEW_COMMIT}),
    )
    .await;
    mount(&server, "POST", "git/refs", 201, json!({})).await;
    mount(&server, "PATCH", "git/refs/heads/hq/fix", 200, json!({})).await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/{REPO}/pulls")))
        .and(query_param("head", "o:hq/fix"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/{REPO}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"default_branch": "main"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/repos/{REPO}/pulls")))
        .and(body_partial_json(json!({"head": "hq/fix", "base": "main"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"html_url": PR_URL})))
        .mount(&server)
        .await;
    server
}

fn tool(server: &MockServer, root: &Path) -> GithubWriteTool {
    GithubWriteTool::for_test(allowed(), format!("{}/", server.uri()), root.to_path_buf())
}

async fn requests_to(server: &MockServer, verb: &str, at: &str) -> Vec<wiremock::Request> {
    let wanted = format!("/repos/{REPO}/{at}");
    let all = server.received_requests().await.unwrap();
    all.into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == wanted)
        .collect()
}

#[tokio::test]
async fn a_first_push_creates_the_branch_from_the_base_commit_and_opens_a_pr() {
    let (root, dest) = checkout();
    fs::write(dest.join("edit.txt"), b"different").unwrap();
    let server = github(None).await;
    let request = Request::parse(&args("fix", "Fix it"), &allowed()).unwrap();
    let reply = tool(&server, root.path())
        .run(request, TOKEN.into())
        .await
        .unwrap();

    assert_eq!(reply["commit"], NEW_COMMIT);
    assert_eq!(reply["created_branch"], true);
    assert_eq!(reply["pull_request"], PR_URL);
    assert_eq!(reply["deleted"], json!(["gone.txt"]));
    let trees = requests_to(&server, "POST", "git/trees").await;
    let tree: Value = serde_json::from_slice(&trees[0].body).unwrap();
    assert_eq!(tree["base_tree"], TREE_SHA);
    assert!(
        tree["tree"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "gone.txt" && e["sha"].is_null())
    );
    let commits = requests_to(&server, "POST", "git/commits").await;
    let commit: Value = serde_json::from_slice(&commits[0].body).unwrap();
    assert_eq!(commit["parents"], json!([BASE_SHA]));
    let refs = requests_to(&server, "POST", "git/refs").await;
    let made: Value = serde_json::from_slice(&refs[0].body).unwrap();
    assert_eq!(made["ref"], "refs/heads/hq/fix");
    let auth = refs[0]
        .headers
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(auth, format!("Bearer {TOKEN}"));
    assert_eq!(
        read_pushed(&pushed_record_path(&dest))["hq/fix"],
        NEW_COMMIT
    );
}

#[tokio::test]
async fn a_second_push_moves_its_own_branch_forward_without_force() {
    let (root, dest) = checkout();
    fs::write(dest.join("edit.txt"), b"different").unwrap();
    let mut record = BTreeMap::new();
    record.insert("hq/fix".to_string(), OTHER_COMMIT.to_string());
    write_pushed(&pushed_record_path(&dest), &record).unwrap();
    let server = github(Some(OTHER_COMMIT)).await;
    let mut a = args("fix", "Fix again");
    a["open_pr"] = json!(false);
    let request = Request::parse(&a, &allowed()).unwrap();
    let reply = tool(&server, root.path())
        .run(request, TOKEN.into())
        .await
        .unwrap();

    assert_eq!(reply["created_branch"], false);
    assert!(reply["pull_request"].is_null());
    let commits = requests_to(&server, "POST", "git/commits").await;
    let commit: Value = serde_json::from_slice(&commits[0].body).unwrap();
    assert_eq!(commit["parents"], json!([OTHER_COMMIT]));
    let patches = requests_to(&server, "PATCH", "git/refs/heads/hq/fix").await;
    let patch: Value = serde_json::from_slice(&patches[0].body).unwrap();
    assert_eq!(patch["force"], false);
}

#[tokio::test]
async fn a_branch_someone_else_pushed_to_is_left_alone() {
    let (root, dest) = checkout();
    fs::write(dest.join("edit.txt"), b"different").unwrap();
    let server = github(Some(OTHER_COMMIT)).await;
    let request = Request::parse(&args("fix", "m"), &allowed()).unwrap();
    let err = tool(&server, root.path())
        .run(request, TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("did not push"), "{err}");
    assert!(requests_to(&server, "POST", "git/commits").await.is_empty());
}

#[tokio::test]
async fn nothing_changed_and_no_checkout_are_refused_before_github_is_called() {
    let (root, dest) = checkout();
    fs::write(dest.join("gone.txt"), b"old").unwrap();
    let server = MockServer::start().await;
    let request = Request::parse(&args("fix", "m"), &allowed()).unwrap();
    let err = tool(&server, root.path())
        .run(request, TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nothing changed"), "{err}");

    let empty = tempfile::tempdir().unwrap();
    let request = Request::parse(&args("fix", "m"), &allowed()).unwrap();
    let err = tool(&server, empty.path())
        .run(request, TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("fetch it first"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}
