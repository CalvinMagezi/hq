# Private Drive images in Google Docs (FR-058)

Inserting a private Drive image into a Doc with `documents.batchUpdate insertInlineImage` fails,
because Google fetches the image URI itself and needs it to be public:

```
400 Invalid requests[0].insertInlineImage: There was a problem retrieving the image.
The provided image should be publicly accessible, within size limit, and in supported formats.
```

`scripts/gdoc_insert_image.py` avoids making the file public. It deploys an Apps Script web app
that runs as the user with access "Only myself"; the script reads the image with `DriveApp` and
inserts it with the Docs service. Sharing never changes.

## Setup (once, needs the owner)

1. Enable the API: `gcloud services enable script.googleapis.com --project <gws project>`.
2. Turn on the Apps Script API at https://script.google.com/home/usersettings.
3. `gdoc_insert_image.py auth` opens a browser for consent (documents, drive.readonly,
   script.projects, script.deployments) using the OAuth client gws already has. The refresh token is
   stored at `~/.config/hq/gdoc-image.json`, mode 0600.
4. `gdoc_insert_image.py setup` creates and deploys the script. Open the printed URL once and approve it.
   The page then shows "Script function not found: doGet", which is expected.

## Use

`gdoc_insert_image.py insert DOC_ID IMAGE_FILE_ID "<anchor text>"` replaces the anchor text with the
image (max width 468 pt) and tags it `hq-img:<fileId>:<anchor>` in its alt title. A repeat run finds the
tag and answers `exists`, so retries never duplicate. Other answers: `inserted`, `anchor_not_found`.

## Notes

- The script's web app answers a POST with a 302 to a result URL. That URL must be fetched with GET and
  without the bearer token; sending the token returns an HTML error page.
- `gws` has no `script` service and does not expose its access token, so this tool signs in itself.
- A Word round trip (export, edit, re-upload) was rejected: it replaces the whole body and loses
  comments, suggestions and formatting.
- Only the first anchor match is used, and images inside tables or headers are not searched for the tag.

## Verified on a scratch Doc (2026-09-30)

Two anchors, each inserted once; a repeat run answered `exists`; a missing anchor answered
`anchor_not_found`; `documents.get` showed exactly two inline images with the expected tags; the image's
permission list was identical before and after; the exported PDF showed both pictures in place.
