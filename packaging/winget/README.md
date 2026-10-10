# Package manager manifests for HQ Lite

Templates, not published manifests. They need a released, hashed Lite zip, which
`.github/workflows/windows-lite.yml` produces. `scripts/windows/render-packaging.ps1` fills them in
from a release's zip and `.sha256` file:

```
pwsh scripts/windows/render-packaging.ps1 -Zip hq-lite-0.9.1-abc1234-windows-x86_64.zip -Url <download url>
```

- **Scoop** installs per user with no administrator rights and does not depend on winget policy:
  `scoop install <path or url of hq-lite.json>`.
- **winget** supports portable zips per user, but Group Policy can disable winget. `manifests/` holds a
  complete multi-file set (version, installer, locale) for the signed build `v0.9.1-main.136`, in the
  layout microsoft/winget-pkgs expects. It is NOT submitted. To submit: copy
  `manifests/h/HQ/Lite/<version>` into a fork of microsoft/winget-pkgs at the same path, run
  `winget validate --manifest <dir>` and a local `winget install --manifest <dir>` on Windows
  (neither has been run; there is no Windows here; check in particular that the `web` folder lands next to `hq.exe`, which `hq web` needs, and use the newest manifest schema version at submit time), then open the pull request. Their reviewers may
  object to a prerelease main-branch build and an unsigned (no Authenticode) executable; a stable
  release is the better first submission. The `hq-lite.yaml` singleton is the render template for
  later versions.
