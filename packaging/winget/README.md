# Windows Package Manager (winget) Manifest for nodewipe

Official Winget manifest for [nodewipe](https://github.com/KADHIRAVANEG/nodewipe).

## Submission to microsoft/winget-pkgs

The official repository for public winget packages is [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).

### Automatic Submission via wingetcreate
On Windows, you can validate and submit directly using Microsoft's official `wingetcreate` CLI tool:

```powershell
# Install wingetcreate
winget install Microsoft.WingetCreate

# Submit new version to microsoft/winget-pkgs
wingetcreate submit https://github.com/KADHIRAVANEG/nodewipe/releases/download/v0.9.0/nodewipe-windows-x86_64.exe
```

### Local Testing
To test the manifest locally without submitting:
```powershell
winget install --manifest packaging/winget/KADHIRAVANEG.nodewipe.yaml
```

## User Installation
Once merged into `microsoft/winget-pkgs`, users can install with:
```powershell
winget install nodewipe
# or:
winget install KADHIRAVANEG.nodewipe
```
