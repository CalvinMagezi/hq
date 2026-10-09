# Package manager manifests for HQ Lite

Templates, not published manifests. They need a released, hashed Lite zip, which
`.github/workflows/windows-lite.yml` produces. `scripts/windows/render-packaging.ps1` fills them in
from a release's zip and `.sha256` file:

```
pwsh scripts/windows/render-packaging.ps1 -Zip hq-lite-0.9.1-abc1234-windows-x86_64.zip -Url <download url>
```

- **Scoop** installs per user with no administrator rights and does not depend on winget policy:
  `scoop install <path or url of hq-lite.json>`.
- **winget** supports portable zips per user, but Group Policy can disable winget. Submitting to the
  community repository is a separate, manual step once builds are signed.
