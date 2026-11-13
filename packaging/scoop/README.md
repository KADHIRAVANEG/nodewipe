# Scoop Package Manifest for nodewipe

Official Scoop manifest for [nodewipe](https://github.com/KADHIRAVANEG/nodewipe).

## Installation via Scoop

### Option 1: Direct URL Installation
```powershell
scoop install https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/packaging/scoop/nodewipe.json
```

### Option 2: Custom Bucket
```powershell
scoop bucket add nodewipe https://github.com/KADHIRAVANEG/scoop-nodewipe
scoop install nodewipe
```

## Updating
```powershell
scoop update nodewipe
```

## Maintenance
To test or update the manifest with new versions:
```powershell
scoop checkver nodewipe -u
```
