#!/usr/bin/env python3
"""Insert a private Google Drive image into a Google Doc without making it public.

The Docs API fetches image URIs itself, so a private Drive file cannot be inserted
through it. This tool provisions a small Apps Script web app that runs as the user
("Only myself"), reads the image with DriveApp and inserts it with the Docs service.
Sharing settings never change.

    gdoc_insert_image.py auth                       one-time browser consent, stores a refresh token
    gdoc_insert_image.py setup                      create and deploy the script (once)
    gdoc_insert_image.py insert DOC IMAGE ANCHOR    replace ANCHOR text with the image, idempotent

Uses the OAuth client gws already has at ~/.config/gws/client_secret.json. The token is
stored at ~/.config/hq/gdoc-image.json with mode 0600. Standard library only.
"""

import base64
import hashlib
import http.server
import json
import os
import secrets
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
import webbrowser

CLIENT_FILE = os.path.expanduser("~/.config/gws/client_secret.json")
STATE_FILE = os.path.expanduser("~/.config/hq/gdoc-image.json")
SCOPES = [
    "https://www.googleapis.com/auth/documents",
    "https://www.googleapis.com/auth/drive.readonly",
    "https://www.googleapis.com/auth/script.projects",
    "https://www.googleapis.com/auth/script.deployments",
]
AUTH_TIMEOUT_SECS = 300
DEFAULT_WIDTH_PT = 468
SCRIPT_TITLE = "hq-doc-image-inserter"
TAG_PREFIX = "hq-img:"

APPS_SCRIPT = r"""
function json_(o) {
  return ContentService.createTextOutput(JSON.stringify(o)).setMimeType(ContentService.MimeType.JSON);
}
function escapeRegex_(s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }
function doPost(e) {
  var p = JSON.parse(e.postData.contents);
  var tag = '%TAG_PREFIX%' + p.imageId + ':' + p.anchor;
  var body = DocumentApp.openById(p.docId).getBody();
  var images = body.getImages();
  for (var i = 0; i < images.length; i++) {
    if (images[i].getAltTitle() === tag) return json_({status: 'exists'});
  }
  var hit = body.findText(escapeRegex_(p.anchor));
  if (!hit) return json_({status: 'anchor_not_found'});
  var text = hit.getElement().asText();
  var paragraph = text.getParent().asParagraph();
  var blob = DriveApp.getFileById(p.imageId).getBlob();
  text.deleteText(hit.getStartOffset(), hit.getEndOffsetInclusive());
  var img = paragraph.appendInlineImage(blob);
  var width = Math.min(img.getWidth(), p.width || %DEFAULT_WIDTH%);
  img.setHeight(Math.round(img.getHeight() * width / img.getWidth())).setWidth(width);
  img.setAltTitle(tag);
  return json_({status: 'inserted'});
}
"""

MANIFEST = {
    "timeZone": "UTC",
    "exceptionLogging": "STACKDRIVER",
    "runtimeVersion": "V8",
    "oauthScopes": [
        "https://www.googleapis.com/auth/documents",
        "https://www.googleapis.com/auth/drive.readonly",
    ],
    "webapp": {"executeAs": "USER_DEPLOYING", "access": "MYSELF"},
}


def client():
    with open(CLIENT_FILE) as f:
        return json.load(f)["installed"]


def load_state():
    try:
        with open(STATE_FILE) as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def save_state(state):
    os.makedirs(os.path.dirname(STATE_FILE), exist_ok=True)
    fd = os.open(STATE_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(state, f)


def token_request(fields):
    c = client()
    data = urllib.parse.urlencode({"client_id": c["client_id"], "client_secret": c["client_secret"], **fields}).encode()
    try:
        return json.load(urllib.request.urlopen(urllib.request.Request(c["token_uri"], data)))
    except urllib.error.HTTPError as e:
        sys.exit(f"token endpoint refused the request: {e.read().decode()[:200]}")


def access_token():
    state = load_state()
    if "refresh_token" not in state:
        sys.exit("not authorized yet: run `gdoc_insert_image.py auth`")
    return token_request({"grant_type": "refresh_token", "refresh_token": state["refresh_token"]})["access_token"]


def cmd_auth():
    verifier = secrets.token_urlsafe(64)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    got = {}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            got.update(urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query))
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"Authorized. You can close this tab.")

        def log_message(self, *_):
            pass

    server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    server.timeout = AUTH_TIMEOUT_SECS
    redirect = f"http://127.0.0.1:{server.server_port}"
    params = {
        "client_id": client()["client_id"], "redirect_uri": redirect, "response_type": "code",
        "scope": " ".join(SCOPES), "access_type": "offline", "prompt": "consent",
        "code_challenge": challenge, "code_challenge_method": "S256",
    }
    url = client()["auth_uri"] + "?" + urllib.parse.urlencode(params)
    print("Opening the consent page. If nothing opens, visit:\n" + url, flush=True)
    webbrowser.open(url)
    threading.Thread(target=server.handle_request).start()
    server.handle_request()
    if "code" not in got:
        sys.exit(f"no authorization code received ({got.get('error', ['timed out'])[0]})")
    tokens = token_request({"grant_type": "authorization_code", "code": got["code"][0],
                            "redirect_uri": redirect, "code_verifier": verifier})
    state = load_state()
    state["refresh_token"] = tokens["refresh_token"]
    save_state(state)
    print("authorized; refresh token saved")


def api(method, url, body=None, headers=None):
    hdrs = {"Authorization": "Bearer " + access_token(), "Content-Type": "application/json", **(headers or {})}
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method=method, headers=hdrs)
    try:
        return json.load(urllib.request.urlopen(req))
    except urllib.error.HTTPError as e:
        sys.exit(f"{method} {url.split('?')[0]} failed {e.code}: {e.read().decode()[:400]}")


def cmd_setup():
    base = "https://script.googleapis.com/v1/projects"
    project = api("POST", base, {"title": SCRIPT_TITLE})
    sid = project["scriptId"]
    source = APPS_SCRIPT.replace("%TAG_PREFIX%", TAG_PREFIX).replace("%DEFAULT_WIDTH%", str(DEFAULT_WIDTH_PT))
    api("PUT", f"{base}/{sid}/content", {"files": [
        {"name": "Code", "type": "SERVER_JS", "source": source},
        {"name": "appsscript", "type": "JSON", "source": json.dumps(MANIFEST)},
    ]})
    version = api("POST", f"{base}/{sid}/versions", {"description": "image inserter"})["versionNumber"]
    dep = api("POST", f"{base}/{sid}/deployments", {"versionNumber": version, "manifestFileName": "appsscript",
                                                     "description": "web app, only myself"})
    url = next(e["webApp"]["url"] for e in dep["entryPoints"] if "webApp" in e)
    state = load_state()
    state.update({"script_id": sid, "web_app_url": url})
    save_state(state)
    print(f"deployed script {sid}\nOpen this URL once in a browser signed in as you, to grant the script access:\n{url}")


class GetWithoutAuth(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # The script has already run when it answers 302. The result URL is fetched with GET and
        # no token, since sending the token makes it return an HTML error page.
        return urllib.request.Request(newurl)


def cmd_insert(doc_id, image_id, anchor):
    url = load_state().get("web_app_url") or sys.exit("not set up: run `gdoc_insert_image.py setup`")
    body = json.dumps({"docId": doc_id, "imageId": image_id, "anchor": anchor}).encode()
    req = urllib.request.Request(url, data=body, method="POST",
                                 headers={"Authorization": "Bearer " + access_token(), "Content-Type": "application/json"})
    opener = urllib.request.build_opener(GetWithoutAuth)
    try:
        raw = opener.open(req).read().decode()
    except urllib.error.HTTPError as e:
        sys.exit(f"web app call failed {e.code}: {e.read().decode()[:300]}")
    try:
        print(json.dumps(json.loads(raw)))
    except json.JSONDecodeError:
        sys.exit("web app did not return JSON (the script probably still needs its first-run consent): " + raw[:200])


def main(argv):
    if argv[:1] == ["auth"]:
        cmd_auth()
    elif argv[:1] == ["setup"]:
        cmd_setup()
    elif argv[:1] == ["insert"] and len(argv) == 4:
        cmd_insert(*argv[1:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
