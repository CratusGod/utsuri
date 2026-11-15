# Headless test of Utsuri's UI (Rune tooling): every window renders, and scripted taps,
# drags, typing and file drops change what they should (checked in the element tree).
# No window opens and nothing touches the screen, mouse or keyboard; settings go to a temp
# file. Keep this file ASCII (Windows PowerShell 5.1).
param([switch]$Release, [switch]$KeepPngs)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root ("target\" + $(if ($env:CRATUS_PROFILE) { $env:CRATUS_PROFILE } elseif ($Release) { "release" } else { "debug" }) + "\utsuri.exe")
$out = Join-Path $root "target\headless-test"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out | Out-Null
$env:UTSURI_SETTINGS = Join-Path $out "settings.json"
$script:pass = 0; $script:fail = 0
. (Join-Path $PSScriptRoot "overflow.ps1")
$X = [string][char]0xD7; $A = [string][char]0x2192

function Check($name, $ok, $detail = "") {
    if ($ok) { $script:pass++; Write-Host "  ok   $name $detail" }
    else { $script:fail++; Write-Host "  FAIL $name $detail" -ForegroundColor Red }
}

function Run([string[]]$a) {
    $p = Start-Process -FilePath $exe -ArgumentList $a -NoNewWindow -PassThru -RedirectStandardError (Join-Path $out "stderr.txt") -RedirectStandardOutput (Join-Path $out "stdout.txt")
    $null = $p.Handle  # keeps ExitCode readable after a timed wait
    # a run that hangs (or sits behind a crash box) is killed and fails, never stalls the suite
    if (-not $p.WaitForExit(90000)) {
        Write-Host "  (killed after 90 s: $($a -join ' '))" -ForegroundColor Red
        try { $p.Kill() } catch {}
        return 124
    }
    $p.WaitForExit()
    return $p.ExitCode
}

# the visible text of every element in a tree, one per line
function TreeText($path) {
    if (-not (Test-Path $path)) { return "" }
    $raw = Get-Content -Raw -Encoding UTF8 $path
    return ([regex]::Matches($raw, '"(?:name|text)":\s*"((?:[^"\\]|\\.)*)"') | ForEach-Object { $_.Groups[1].Value }) -join "`n"
}

function Script($window, $name, $events, $at, [string[]]$expect, [string[]]$absent = @()) {
    $sp = Join-Path $out "$name.json"
    ('{"events": [' + ($events -join ",") + ']}') | Out-File -Encoding ascii $sp
    $tree = Join-Path $out "$name.tree.json"
    $code = Run @("--window", $window, "--script", $sp, "--tree", $tree, "--at", "$at")
    $t = TreeText $tree
    $missing = @($expect | Where-Object { $t -notlike "*$_*" })
    $present = @($absent | Where-Object { $t -like "*$_*" })
    $over = @(Get-Overflows $tree)
    Check "$window / $name" ($code -eq 0 -and $missing.Count -eq 0 -and $present.Count -eq 0 -and $over.Count -eq 0) $(if ($missing.Count) { "missing: " + ($missing -join " | ") } elseif ($present.Count) { "unexpected: " + ($present -join " | ") } elseif ($over.Count) { "text cut off: " + ($over -join " | ") } else { "" })
}

Write-Host "== windows render"
Run @("--list-windows") | Out-Null
$list = Get-Content (Join-Path $out "stdout.txt") | Where-Object { $_ -and $_ -notmatch "^rune:" }
$want = @("menu", "menu-tools", "menu-hub", "menu-click", "toasts", "toast-update", "settings", "welcome", "compress", "edit", "crop", "redact", "annotate", "addbg", "stamp", "organize", "resize", "metadata", "pdf-metadata", "protect", "unlock")
foreach ($w in $want) {
    if ($list -notcontains $w) { Check "listed: $w" $false; continue }
    $png = Join-Path $out "$w.png"
    $code = Run @("--window", $w, "--snap", $png, "--at", "1.5")
    Check "renders: $w" ($code -eq 0 -and (Test-Path $png) -and (Get-Item $png).Length -gt 5000) ("{0:N0} bytes" -f $(if (Test-Path $png) { (Get-Item $png).Length } else { 0 }))
}

Write-Host "== nothing cut off"
foreach ($w in $want) { Script $w "layout-$w" @() 1.0 @() }

Write-Host "== menu and notifications"
Script "menu" "convert-ring" @() 0.5 @("Convert", "WEBP", "Smallest for web", "photo.jpg", "PNG", "PDF")
Script "menu-tools" "tools-ring" @() 0.5 @("Tools", "Compress", "Crop", "Annotate", "Trim to a ratio")
Script "menu-hub" "pdf-ring" @() 0.5 @("report.pdf")
Script "toasts" "cards" @() 0.5 @("JPG", "75% smaller", "Open", "Show in folder", "Undo", "Compressing", "Couldn't finish")

Write-Host "== tools"
Script "resize" "preset-25" @('{"t":0.2,"type":"tap","key":"preset/0"}') 0.6 @("1600 $X 1067  $A  400 $X 267")
Script "resize" "typed-width" @('{"t":0.2,"type":"tap","key":"w"}', '{"t":0.3,"type":"key","name":"End"}', '{"t":0.35,"type":"key","name":"Backspace"}', '{"t":0.4,"type":"key","name":"Backspace"}', '{"t":0.45,"type":"key","name":"Backspace"}', '{"t":0.5,"type":"type","text":"1000"}') 1.4 @("1600 $X 1067  $A  1000 $X 667")
Script "compress" "target-mode" @('{"t":0.2,"type":"tap","key":"mode/1"}') 3.0 @("Make each under", "quality")
Script "crop" "square" @('{"t":0.2,"type":"tap","key":"ratio/2"}') 0.6 @("1067 $X 1067 px")
Script "crop" "rotate" @('{"t":0.2,"type":"tap","key":"rotate-right"}') 0.6 @("1067 $X 1600 px")
Script "redact" "one-box" @('{"t":0.2,"type":"drag","x":300,"y":320,"dx":160,"dy":90,"dur":0.3}') 1.0 @("1 area")
Script "redact" "undo" @('{"t":0.2,"type":"drag","x":300,"y":320,"dx":160,"dy":90,"dur":0.3}', '{"t":0.8,"type":"tap","key":"undo"}') 1.2 @("Drag over what to hide")
Script "annotate" "text" @('{"t":0.2,"type":"tap","key":"tool/5"}', '{"t":0.3,"type":"tap","x":400,"y":400}', '{"t":0.5,"type":"type","text":"Hello"}', '{"t":1.0,"type":"key","name":"Enter"}') 1.5 @("Hello")
Script "protect" "match" @('{"t":0.2,"type":"tap","key":"pw"}', '{"t":0.3,"type":"type","text":"Secret1!"}', '{"t":0.9,"type":"tap","key":"pw2"}', '{"t":1.0,"type":"type","text":"Secret1!"}') 1.8 @("Looks good")
Script "protect" "mismatch" @('{"t":0.2,"type":"tap","key":"pw"}', '{"t":0.3,"type":"type","text":"Secret1!"}', '{"t":0.9,"type":"tap","key":"pw2"}', '{"t":1.0,"type":"type","text":"nope"}') 1.6 @("don't match")
Script "unlock" "prompt" @() 0.5 @("Password", "Unlock")
Script "metadata" "gps" @('{"t":0.2,"type":"tap","key":"all-toggle"}') 0.6 @("includes your GPS location", "Location", "Remove location only", "All fields")
Script "pdf-metadata" "fields" @() 0.5 @("hidden field")
Script "stamp" "image-mode" @('{"t":0.2,"type":"tap","key":"mode/1"}') 0.6 @("Choose image", "Pick an image to stamp")
Script "stamp" "bad-pages" @('{"t":0.2,"type":"tap","key":"pages"}', '{"t":0.3,"type":"type","text":"9-12"}') 0.8 @("9")
Script "organize" "delete-page" @('{"t":0.2,"type":"tap","key":"page/0"}', '{"t":0.4,"type":"tap","key":"delete"}') 0.8 @("7  ") @("8  ")
Script "organize" "cut" @('{"t":0.2,"type":"tap","key":"page/3"}', '{"t":0.4,"type":"tap","key":"cut"}') 0.8 @("Save 2 PDFs")
Script "organize" "pick-pages" @('{"t":0.2,"type":"tap","key":"pick"}', '{"t":0.3,"type":"type","text":"2-3, 6"}', '{"t":0.9,"type":"key","name":"Enter"}') 1.5 @("3 selected", "Save")
Script "organize" "pick-bad" @('{"t":0.2,"type":"tap","key":"pick"}', '{"t":0.3,"type":"type","text":"40"}', '{"t":0.6,"type":"key","name":"Enter"}') 1.2 @("Use page numbers from 1 to 8")
Script "organize" "extract-shown" @('{"t":0.2,"type":"tap","key":"page/1"}') 0.6 @("Extract selection", "1 selected")
Script "addbg" "aspect" @('{"t":0.2,"type":"tap","key":"aspect/1"}', '{"t":0.3,"type":"tap","key":"preset/3"}') 1.0 @("Save with Background", "Copy")
Script "settings" "trigger" @('{"t":0.2,"type":"tap","key":"trigger/1"}', '{"t":0.4,"type":"tap","key":"dpi/3"}') 0.8 @("Ctrl + drag", "300 dpi")
$saved = if (Test-Path $env:UTSURI_SETTINGS) { Get-Content -Raw $env:UTSURI_SETTINGS | ConvertFrom-Json } else { $null }
Check "settings saved to the test file" ($saved -and $saved.trigger -eq "ctrl" -and $saved.pdf_dpi -eq 300)

Write-Host "== updates"
Script "toast-update" "update-card" @() 0.5 @("Utsuri 2.0.1 is available", "Update now?", "Update", "Later")
Script "settings" "update-controls" @() 0.5 @("Check for updates automatically", "Check for updates")
Script "settings" "updates-off" @('{"t":0.2,"type":"tap","key":"updates"}') 0.6 @()
$saved = if (Test-Path $env:UTSURI_SETTINGS) { Get-Content -Raw $env:UTSURI_SETTINGS | ConvertFrom-Json } else { $null }
Check "turning automatic update checks off is saved" ($saved -and $saved.check_updates -eq $false)

if (-not $KeepPngs) { Get-ChildItem $out -Filter *.json | Remove-Item }
Write-Host ""
Write-Host "$($script:pass) passed, $($script:fail) failed  (renders in $out)"
if ($script:fail -gt 0) { exit 1 }
