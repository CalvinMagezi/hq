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
const OUR_EARLIER: &str = "5555555555555555555555555555555555555555";
const BLOB_SHA: &str = "6666666666666666666666666666666666666666";
const OLD_TREE: &str = "7777777777777777777777777777777777777777";
const PR_URL: &str = "https://github.com/o/r/pull/7";
const TOKEN: &str = "github_pat_11TESTTOKEN0123456789_abcdefghijklmnop";

fn base_file(path: &str, content: &[u8]) -> BaseFile {
    BaseFile {
        path: path.into(),
        mode: "100644".into(),
        sha256: hex::encode(Sha256::digest(content)),
        size: content.len() as u64,
        known_secrets: crate::github::tarball::secret_hashes(content),
    }
}

/// A checkout of o/r fetched at BASE_SHA with `keep.txt`, `edit.txt` and `gone.txt`, and its
/// records kept apart as in production.
struct Fixture {
    root: tempfile::TempDir,
    dest: PathBuf,
    records: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        Self::with(&[])
    }

    fn with(extra: &[BaseFile]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("clones").join(REPO);
        let records = root.path().join("records").join(REPO);
        fs::create_dir_all(&dest).unwrap();
        fs::create_dir_all(&records).unwrap();
        for name in ["keep.txt", "edit.txt"] {
            fs::write(dest.join(name), b"same").unwrap();
        }
        let mut files = vec![
            base_file("edit.txt", b"same"),
            base_file("gone.txt", b"old"),
            base_file("keep.txt", b"same"),
        ];
        files.extend(extra.iter().cloned());
        let base = Base {
            repo: REPO.into(),
            commit: BASE_SHA.into(),
            files,
        };
        fs::write(
            base_record_path(&records),
            serde_json::to_vec(&base).unwrap(),
        )
        .unwrap();
        Self {
            root,
            dest,
            records,
        }
    }

    fn base(&self) -> Base {
        load_base(&self.records).unwrap()
    }

    fn changes(&self) -> Result<Vec<Change>> {
        changes(
            &self.dest,
            &self.root.path().join("clones"),
            &self.base(),
            &[TOKEN.to_string()],
        )
    }

    fn tool(&self, server: &MockServer) -> GithubWriteTool {
        GithubWriteTool::for_test(
            allowed(),
            format!("{}/", server.uri()),
            self.root.path().join("clones"),
            self.root.path().join("records"),
        )
    }
}

fn args(branch: &str, message: &str) -> Value {
    json!({"repo": REPO, "branch": branch, "message": message})
}

fn allowed() -> Vec<String> {
    vec![REPO.to_string()]
}

fn parse(a: &Value) -> Result<Request> {
    Request::parse(a, &allowed())
}

#[test]
fn only_allowlisted_repositories_and_plain_branch_names_pass() {
    let err = Request::parse(&args("x", "m"), &[]).err().unwrap();
    assert!(err.to_string().contains("github_write.repos"));
    assert!(
        Request::parse(
            &json!({"repo": "O/R", "branch": "x", "message": "m"}),
            &allowed()
        )
        .is_ok()
    );
    for bad in [
        "../x", "-x", "a//b", "x.lock", "a.lock/b", "fix.", "a..b", "a b", "/x", "x/", "", "x?y",
        "x#y",
    ] {
        assert!(parse(&args(bad, "m")).is_err(), "{bad:?}");
    }
    let ok = parse(&args("hq/fix-log", "Fix the log\n\nDetails")).unwrap();
    assert_eq!(ok.branch, "hq/fix-log", "a given hq/ prefix is not doubled");
    assert_eq!(ok.title, "Fix the log");
    assert!(ok.open_pr);
}

#[test]
fn credentials_and_oversized_text_are_refused_before_anything_is_pushed() {
    let key = format!("sk-ant-api03-{}", "a1".repeat(20));
    for field in ["message", "title", "body"] {
        let mut a = args("x", "m");
        a[field] = json!(format!("use {key}"));
        let err = parse(&a).err().unwrap().to_string();
        assert!(err.contains(&format!("`{field}`")), "{err}");
    }
    let mut a = args("x", "m");
    a["title"] = json!("t".repeat(MAX_TITLE_CHARS + 1));
    assert!(parse(&a).is_err());
}

#[test]
fn the_live_token_is_refused_in_text_even_when_its_shape_is_unknown() {
    let request = parse(&args("x", "the token is plain-looking-value-123")).unwrap();
    assert!(
        request
            .refuse_secret(&["plain-looking-value-123".to_string()])
            .is_err()
    );
}

#[test]
fn changes_find_new_changed_and_deleted_files_with_their_modes() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    fs::create_dir_all(f.dest.join("src")).unwrap();
    fs::write(f.dest.join("src/new.rs"), b"fn main() {}").unwrap();
    let found = f.changes().unwrap();
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
fn links_of_every_kind_refuse_the_push_and_are_never_read() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("config.yaml"), b"secret").unwrap();

    let f = Fixture::new();
    std::os::unix::fs::symlink(
        outside.path().join("config.yaml"),
        f.dest.join("file-link.txt"),
    )
    .unwrap();
    assert!(f.changes().unwrap_err().to_string().contains("link"));

    let f = Fixture::new();
    std::os::unix::fs::symlink(outside.path(), f.dest.join("dir-link")).unwrap();
    assert!(f.changes().unwrap_err().to_string().contains("link"));

    let f = Fixture::new();
    fs::hard_link(outside.path().join("config.yaml"), f.dest.join("hard.txt")).unwrap();
    assert!(f.changes().unwrap_err().to_string().contains("single name"));

    let f = Fixture::new();
    let moved = f.root.path().join("real");
    fs::rename(&f.dest, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &f.dest).unwrap();
    assert!(
        f.changes()
            .unwrap_err()
            .to_string()
            .contains("plain folder")
    );
}

#[test]
fn git_metadata_workflows_and_credentials_refuse_the_push() {
    let f = Fixture::new();
    fs::create_dir_all(f.dest.join(".git/hooks")).unwrap();
    assert!(
        f.changes()
            .unwrap_err()
            .to_string()
            .contains("git metadata")
    );

    let f = Fixture::new();
    fs::create_dir_all(f.dest.join(".github/workflows")).unwrap();
    fs::write(f.dest.join(".github/workflows/ci.yml"), b"on: push").unwrap();
    assert!(f.changes().unwrap_err().to_string().contains(".github/"));

    let f = Fixture::new();
    let pem = "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----\n";
    fs::write(f.dest.join("leak.txt"), pem).unwrap();
    let err = f.changes().unwrap_err().to_string();
    assert!(
        err.contains("leak.txt") && err.contains("credential"),
        "{err}"
    );
    assert!(
        !err.contains("BEGIN RSA"),
        "the refusal never repeats the value"
    );

    let f = Fixture::new();
    fs::write(f.dest.join("notes.txt"), format!("remember {TOKEN} later")).unwrap();
    assert!(f.changes().unwrap_err().to_string().contains("notes.txt"));

    let f = Fixture::new();
    fs::write(f.dest.join("ghp_abcdefghijklmnopqrstuvwxyz0123"), b"x").unwrap();
    assert!(
        f.changes().unwrap_err().to_string().contains("credential"),
        "a name is published too"
    );
}

#[test]
fn a_file_that_already_held_a_credential_shape_may_still_be_edited() {
    let fixture = "TEST_API_TOKEN=abcd1234abcd1234abcd\n";
    let f = Fixture::with(&[base_file(".env.example", fixture.as_bytes())]);
    fs::write(
        f.dest.join(".env.example"),
        format!("{fixture}# a comment\n"),
    )
    .unwrap();
    let found = f.changes().unwrap();
    assert!(found.iter().any(|c| c.path == ".env.example"));

    let f = Fixture::with(&[base_file(".env.example", fixture.as_bytes())]);
    fs::write(f.dest.join(".env.example"), format!("{fixture}{TOKEN}\n")).unwrap();
    assert!(
        f.changes().is_err(),
        "the live token is refused even in a flagged file"
    );

    let f = Fixture::with(&[base_file(".env.example", fixture.as_bytes())]);
    let added = format!("sk-ant-api03-{}", "z9".repeat(20));
    fs::write(
        f.dest.join(".env.example"),
        format!("{fixture}OTHER={added}\n"),
    )
    .unwrap();
    assert!(
        f.changes().is_err(),
        "a new credential shape in a flagged file is refused"
    );
}

#[test]
fn a_checkout_that_lost_most_of_its_files_is_not_pushed_as_deletions() {
    let many: Vec<BaseFile> = (0..40)
        .map(|i| base_file(&format!("f{i}.txt"), b"x"))
        .collect();
    let f = Fixture::with(&many);
    fs::write(f.dest.join("gone.txt"), b"old").unwrap();
    let err = f.changes().unwrap_err().to_string();
    assert!(err.contains("fetch again"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_checkout_that_resolves_outside_the_clone_folder_is_refused() {
    let f = Fixture::new();
    let elsewhere = tempfile::tempdir().unwrap();
    let owner = f.root.path().join("clones/o");
    fs::rename(&owner, elsewhere.path().join("o")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path().join("o"), &owner).unwrap();
    let err = f.changes().unwrap_err().to_string();
    assert!(err.contains("outside the clone folder"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_pipe_where_a_file_was_never_hangs_the_read() {
    let f = Fixture::new();
    let pipe = f.dest.join("pipe");
    let c_path = std::ffi::CString::new(pipe.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path and a plain mode.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let dest = f.dest.canonicalize().unwrap();
    std::thread::spawn(move || {
        let _ = tx.send(super::super::changes::read_plain_for_test(&dest, "pipe").is_err());
    });
    let refused = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the read hung on a pipe");
    assert!(refused);
}

#[test]
fn only_a_changed_file_is_held_to_the_push_size_cap() {
    let big = vec![b'x'; MIB_TEST * 6];
    let f = Fixture::with(&[base_file("big.bin", &big)]);
    fs::write(f.dest.join("big.bin"), &big).unwrap();
    fs::write(f.dest.join("gone.txt"), b"old").unwrap();
    assert!(
        f.changes().unwrap().is_empty(),
        "an unchanged big file does not block a push"
    );

    let mut grown = big.clone();
    grown.push(b'y');
    fs::write(f.dest.join("big.bin"), &grown).unwrap();
    assert!(f.changes().unwrap_err().to_string().contains("larger than"));
}

const MIB_TEST: usize = 1024 * 1024;

#[cfg(unix)]
#[test]
fn a_mode_change_alone_counts_as_a_change() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::set_permissions(f.dest.join("keep.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    let found = f.changes().unwrap();
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

/// GitHub for one push. `head` is where `hq/fix` points (absent when `None`) and its tree.
async fn github(head: Option<(&str, &str)>, patch_status: u16) -> MockServer {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        &format!("git/commits/{BASE_SHA}"),
        200,
        json!({"tree": {"sha": TREE_SHA}}),
    )
    .await;
    match head {
        Some((sha, tree)) => {
            mount(
                &server,
                "GET",
                "git/ref/heads/hq/fix",
                200,
                json!({"object": {"sha": sha}}),
            )
            .await;
            mount(
                &server,
                "GET",
                &format!("git/commits/{sha}"),
                200,
                json!({"tree": {"sha": tree}}),
            )
            .await;
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
    mount(
        &server,
        "PATCH",
        "git/refs/heads/hq/fix",
        patch_status,
        json!({"message": "Update is not a fast forward"}),
    )
    .await;
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

async fn bodies(server: &MockServer, verb: &str, at: &str) -> Vec<Value> {
    let wanted = format!("/repos/{REPO}/{at}");
    let all = server.received_requests().await.unwrap();
    all.into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == wanted)
        .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
        .collect()
}

#[tokio::test]
async fn a_first_push_creates_the_branch_from_the_base_commit_and_opens_a_pr() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    let server = github(None, 200).await;
    let reply = f
        .tool(&server)
        .run(parse(&args("fix", "Fix it")).unwrap(), TOKEN.into())
        .await
        .unwrap();

    assert_eq!(reply["commit"], NEW_COMMIT);
    assert_eq!(reply["created_branch"], true);
    assert_eq!(reply["pull_request"], PR_URL);
    assert_eq!(reply["deleted"], json!(["gone.txt"]));
    let blob = &bodies(&server, "POST", "git/blobs").await[0];
    assert_eq!(blob["encoding"], "base64");
    assert_eq!(
        blob["content"],
        base64::engine::general_purpose::STANDARD.encode(b"different")
    );
    let tree = &bodies(&server, "POST", "git/trees").await[0];
    assert_eq!(tree["base_tree"], TREE_SHA);
    let entries = tree["tree"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .all(|e| e["type"] == "blob" && e["mode"] == "100644")
    );
    assert!(
        entries
            .iter()
            .any(|e| e["path"] == "gone.txt" && e["sha"].is_null())
    );
    assert_eq!(
        bodies(&server, "POST", "git/commits").await[0]["parents"],
        json!([BASE_SHA])
    );
    assert_eq!(
        bodies(&server, "POST", "git/refs").await[0]["ref"],
        "refs/heads/hq/fix"
    );
    assert!(
        bodies(&server, "PATCH", "git/refs/heads/hq/fix")
            .await
            .is_empty()
    );
    assert_eq!(read_pushed(&f.records)["hq/fix"], [NEW_COMMIT]);
}

#[tokio::test]
async fn a_second_push_moves_its_own_branch_forward_without_force() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    remember_pushed(&f.records, "hq/fix", OUR_EARLIER).unwrap();
    let server = github(Some((OUR_EARLIER, OLD_TREE)), 200).await;
    let mut a = args("fix", "Fix again");
    a["open_pr"] = json!(false);
    let reply = f
        .tool(&server)
        .run(parse(&a).unwrap(), TOKEN.into())
        .await
        .unwrap();

    assert_eq!(reply["created_branch"], false);
    assert!(reply["pull_request"].is_null());
    assert_eq!(
        bodies(&server, "POST", "git/commits").await[0]["parents"],
        json!([OUR_EARLIER])
    );
    assert_eq!(
        bodies(&server, "PATCH", "git/refs/heads/hq/fix").await[0]["force"],
        false
    );
    assert_eq!(read_pushed(&f.records)["hq/fix"], [OUR_EARLIER, NEW_COMMIT]);
}

#[tokio::test]
async fn a_push_identical_to_the_branch_adds_no_commit() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    remember_pushed(&f.records, "hq/fix", OUR_EARLIER).unwrap();
    let server = github(Some((OUR_EARLIER, NEW_TREE)), 200).await;
    let reply = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap();
    assert_eq!(reply["unchanged"], true);
    assert!(bodies(&server, "POST", "git/commits").await.is_empty());
}

#[tokio::test]
async fn a_branch_someone_else_pushed_to_is_left_alone() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    remember_pushed(&f.records, "hq/fix", OUR_EARLIER).unwrap();
    let theirs = "8888888888888888888888888888888888888888";
    let server = github(Some((theirs, OLD_TREE)), 200).await;
    let err = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("did not push"), "{err}");
    assert!(bodies(&server, "POST", "git/commits").await.is_empty());
    assert!(bodies(&server, "POST", "git/blobs").await.is_empty());
}

#[tokio::test]
async fn a_refused_branch_move_leaves_the_commit_recorded_so_the_next_push_still_works() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    remember_pushed(&f.records, "hq/fix", OUR_EARLIER).unwrap();
    let server = github(Some((OUR_EARLIER, OLD_TREE)), 422).await;
    let err = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("422"), "{err}");
    assert_eq!(read_pushed(&f.records)["hq/fix"], [OUR_EARLIER, NEW_COMMIT]);
}

#[tokio::test]
async fn nothing_changed_no_checkout_and_a_leaked_token_are_refused_before_github_is_called() {
    let f = Fixture::new();
    fs::write(f.dest.join("gone.txt"), b"old").unwrap();
    let server = MockServer::start().await;
    let err = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("nothing changed"), "{err}");

    let empty = Fixture::new();
    fs::remove_file(base_record_path(&empty.records)).unwrap();
    let err = empty
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("fetch it first"), "{err}");

    let request = parse(&args("fix", "plain words")).unwrap();
    let err = f
        .tool(&server)
        .run(request, "plain".into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("credential"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_retry_after_the_pr_failed_opens_it_without_a_new_commit() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    remember_pushed(&f.records, "hq/fix", OUR_EARLIER).unwrap();
    let server = github(Some((OUR_EARLIER, NEW_TREE)), 200).await;
    let reply = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap();
    assert_eq!(reply["unchanged"], true);
    assert_eq!(reply["pull_request"], PR_URL);
    assert!(bodies(&server, "POST", "git/commits").await.is_empty());
}

#[tokio::test]
async fn a_pushed_record_that_is_not_ours_to_trust_claims_no_branch() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    fs::write(f.records.join(PUSHED_RECORD_FILE), b"[\"not\", \"a map\"]").unwrap();
    let server = github(Some((OUR_EARLIER, OLD_TREE)), 200).await;
    let err = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("did not push"), "{err}");
}

#[tokio::test]
async fn a_redirect_is_never_followed_with_the_write_token() {
    let f = Fixture::new();
    fs::write(f.dest.join("edit.txt"), b"different").unwrap();
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/{REPO}/git/commits/{BASE_SHA}")))
        .respond_with(
            ResponseTemplate::new(301).insert_header("location", elsewhere.uri().as_str()),
        )
        .mount(&server)
        .await;
    let err = f
        .tool(&server)
        .run(parse(&args("fix", "m")).unwrap(), TOKEN.into())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("301"), "{err}");
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
}
