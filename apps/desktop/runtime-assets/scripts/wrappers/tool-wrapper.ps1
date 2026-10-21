if ($args.Count -lt 1) {
  [Console]::Error.WriteLine("Missing tool name.")
  exit 64
}

$Tool = [string]$args[0]
$ToolArgs = @()
if ($args.Count -gt 1) {
  $ToolArgs = @($args[1..($args.Count - 1)] | ForEach-Object { [string]$_ })
}

$ErrorActionPreference = "SilentlyContinue"
$wrapperDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$wrapperBin = Join-Path $wrapperDir "bin"

function Write-Live-Log([string]$Message) {
  if ([string]::IsNullOrWhiteSpace($env:DMUX_LOG_FILE)) { return }
  try {
    $parent = Split-Path -Parent $env:DMUX_LOG_FILE
    if (-not [string]::IsNullOrWhiteSpace($parent)) {
      New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $stamp = Get-Date -Format "yyyy-MM-ddTHH:mm:sszzz"
    Add-Content -LiteralPath $env:DMUX_LOG_FILE -Value "[$stamp] [wrapper] $Message"
  } catch {
  }
}

function Split-PathList([string]$Value) {
  if ([string]::IsNullOrWhiteSpace($Value)) { return @() }
  return $Value -split [Regex]::Escape([IO.Path]::PathSeparator)
}

function Join-PathList([string[]]$Values) {
  return ($Values | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }) -join [IO.Path]::PathSeparator
}

function Normalize-Directory([string]$Value) {
  try {
    return [IO.DirectoryInfo]::new($Value).FullName.TrimEnd('\', '/')
  } catch {
    return $Value.TrimEnd('\', '/')
  }
}

function Same-Directory([string]$A, [string]$B) {
  $left = Normalize-Directory $A
  $right = Normalize-Directory $B
  return [string]::Equals($left, $right, [StringComparison]::OrdinalIgnoreCase)
}

function Filter-Wrapper-Path([string]$Value) {
  $parts = Split-PathList $Value
  $filtered = @()
  foreach ($part in $parts) {
    if ([string]::IsNullOrWhiteSpace($part)) { continue }
    if (Same-Directory $part $wrapperBin) { continue }
    $normalized = (Normalize-Directory $part).Replace('\', '/')
    if ($normalized.EndsWith("/runtime-root/scripts/wrappers/bin", [StringComparison]::OrdinalIgnoreCase)) { continue }
    $filtered += $part
  }
  return Join-PathList $filtered
}

function Find-Real-Binary([string]$Name, [string]$SearchPath) {
  $previousPath = $env:PATH
  try {
    $env:PATH = $SearchPath
    $candidateNames = switch ($Name) {
      "claude" { @("claude.ps1", "claude.exe", "claude-code.ps1", "claude-code.exe", "reclaude.ps1", "reclaude.exe"); break }
      "claude-code" { @("claude-code.ps1", "claude-code.exe", "claude.ps1", "claude.exe", "reclaude.ps1", "reclaude.exe"); break }
      "reclaude" { @("reclaude.ps1", "reclaude.exe", "claude.ps1", "claude.exe", "claude-code.ps1", "claude-code.exe"); break }
      "kimi" { @("kimi.ps1", "kimi.exe", "kimi-code.ps1", "kimi-code.exe"); break }
      "kimi-code" { @("kimi-code.ps1", "kimi-code.exe", "kimi.ps1", "kimi.exe"); break }
      default { @("$Name.ps1", "$Name.exe") }
    }
    foreach ($candidate in $candidateNames) {
      $commands = @(Get-Command $candidate -CommandType Application,ExternalScript -ErrorAction SilentlyContinue)
      foreach ($command in $commands) {
        if ($command -and $command.Source -and -not (Same-Directory (Split-Path -Parent $command.Source) $wrapperBin)) {
          return $command.Source
        }
      }
    }
  } finally {
    $env:PATH = $previousPath
  }
  return $null
}

function Read-Tool-Settings {
  # Resolution chain: managed cache file (temp dir, can vanish or be
  # mid-write) -> app support settings.json (source of truth, always
  # readable) -> $null. Every branch logs so a silent "no injection" is
  # diagnosable from DMUX_LOG_FILE. The fallback returns the
  # ai.runtimeTools node, which has the same flat camelCase keys as the
  # managed cache, so downstream $settings.<key> accesses are unchanged.
  $managed = $env:DMUX_TOOL_PERMISSION_SETTINGS_FILE
  if (-not [string]::IsNullOrWhiteSpace($managed)) {
    if (Test-Path -LiteralPath $managed) {
      try {
        $parsed = Get-Content -LiteralPath $managed -Raw | ConvertFrom-Json
        if ($null -ne $parsed) {
          Write-Live-Log "tool settings source=managed path=$managed"
          return $parsed
        }
        Write-Live-Log "tool settings managed cache empty path=$managed"
      } catch {
        Write-Live-Log "tool settings managed cache parse failure path=$managed error=$($_.Exception.Message)"
      }
    } else {
      Write-Live-Log "tool settings managed cache missing path=$managed"
    }
  }
  $supportRoot = $env:DMUX_APP_SUPPORT_ROOT
  if (-not [string]::IsNullOrWhiteSpace($supportRoot)) {
    $fallback = Join-Path $supportRoot "settings.json"
    if (Test-Path -LiteralPath $fallback) {
      try {
        $root = Get-Content -LiteralPath $fallback -Raw | ConvertFrom-Json
        $node = $null
        if ($null -ne $root -and $null -ne $root.ai) {
          $node = $root.ai.runtimeTools
        }
        if ($null -ne $node) {
          Write-Live-Log "tool settings source=app-support path=$fallback"
          return $node
        }
        Write-Live-Log "tool settings app-support node missing path=$fallback"
      } catch {
        Write-Live-Log "tool settings app-support parse failure path=$fallback error=$($_.Exception.Message)"
      }
    } else {
      Write-Live-Log "tool settings app-support missing path=$fallback"
    }
  }
  Write-Live-Log "tool settings source=none tool=$Tool (no tool-permission injection)"
  return $null
}

function Tool-Memory-Injection-Strategy([string]$Name) {
  $path = Join-Path $wrapperDir "tool-drivers.json"
  if (-not (Test-Path -LiteralPath $path)) { return "" }
  try {
    $payload = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    foreach ($driver in @($payload.tools)) {
      $aliases = @($driver.aliases | ForEach-Object { [string]$_ })
      if ($aliases -contains $Name.ToLowerInvariant()) {
        return [string]$driver.memoryInjection
      }
    }
  } catch {
  }
  return ""
}

function Apply-Managed-Lifecycle-Env([string]$Name) {
  $path = Join-Path $wrapperDir "managed-env\$Name.ps1"
  if (Test-Path -LiteralPath $path) {
    . $path
    Write-Live-Log "managed lifecycle env tool=$Name path=$path"
  }
}

function Tool-Config-Key([string]$Name) {
  switch ($Name) {
    "codex" { "codex" }
    "claude" { "claudeCode" }
    "claude-code" { "claudeCode" }
    "reclaude" { "claudeCode" }
    "agy" { "agy" }
    "omp" { "omp" }
    "kimi" { "kimi" }
    "kimi-code" { "kimi" }
    "opencode" { "opencode" }
    "mimo" { "mimo" }
    "kiro-cli" { "kiro" }
    "codewhale" { "codewhale" }
    default { "" }
  }
}

function Tool-Model-Key([string]$Name) {
  switch ($Name) {
    "codex" { "codexModel" }
    "claude" { "claudeCodeModel" }
    "claude-code" { "claudeCodeModel" }
    "reclaude" { "claudeCodeModel" }
    "agy" { "agyModel" }
    "omp" { "ompModel" }
    "kimi" { "kimiModel" }
    "kimi-code" { "kimiModel" }
    "opencode" { "opencodeModel" }
    "mimo" { "mimoModel" }
    "kiro-cli" { "kiroModel" }
    "codewhale" { "codewhaleModel" }
    default { "" }
  }
}

function Has-Arg([string[]]$CommandArgs, [string]$Name) {
  return $CommandArgs -contains $Name
}

function Test-Elevated-Process {
  try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  } catch {
    return $false
  }
}

function Has-Prefix-Arg([string[]]$CommandArgs, [string]$Prefix) {
  foreach ($arg in $CommandArgs) {
    if ($arg.StartsWith($Prefix, [StringComparison]::Ordinal)) { return $true }
  }
  return $false
}

function Has-Option-Value([string[]]$CommandArgs, [string[]]$Names) {
  for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
    if ($Names -contains $CommandArgs[$index]) { return $true }
    foreach ($name in $Names) {
      if ($CommandArgs[$index].StartsWith("$name=", [StringComparison]::Ordinal)) { return $true }
    }
  }
  return $false
}

function Codex-Allows-Additional-Writable-Roots([string[]]$CommandArgs) {
  for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
    $arg = $CommandArgs[$index]
    if ($arg -eq "--dangerously-bypass-approvals-and-sandbox" -or $arg -eq "--full-auto") {
      return $true
    }
    if ($arg -eq "--sandbox" -or $arg -eq "-s") {
      if ($index + 1 -lt $CommandArgs.Count) {
        $value = $CommandArgs[$index + 1]
        if ($value -eq "workspace-write" -or $value -eq "danger-full-access") { return $true }
      }
    }
    if ($arg -eq "--sandbox=workspace-write" -or
        $arg -eq "--sandbox=danger-full-access" -or
        $arg -eq "-sworkspace-write" -or
        $arg -eq "-sdanger-full-access") {
      return $true
    }
  }
  return $false
}

function Codex-Has-Sandbox-Mode-Arg([string[]]$CommandArgs) {
  foreach ($arg in $CommandArgs) {
    if ($arg -eq "--dangerously-bypass-approvals-and-sandbox" -or
        $arg -eq "--full-auto" -or
        $arg -eq "--sandbox" -or
        $arg -eq "-s" -or
        $arg.StartsWith("--sandbox=", [StringComparison]::Ordinal) -or
        $arg.StartsWith("-s", [StringComparison]::Ordinal)) {
      return $true
    }
  }
  return $false
}

function Has-Config-Key([string[]]$CommandArgs, [string]$Key) {
  for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
    $arg = $CommandArgs[$index]
    if (($arg -eq "-c" -or $arg -eq "--config") -and $index + 1 -lt $CommandArgs.Count) {
      if ($CommandArgs[$index + 1].StartsWith("$Key=", [StringComparison]::Ordinal)) { return $true }
    }
    if ($arg.StartsWith("-c$Key=", [StringComparison]::Ordinal) -or
        $arg.StartsWith("--config=$Key=", [StringComparison]::Ordinal)) {
      return $true
    }
  }
  return $false
}

function Extract-Model([string[]]$CommandArgs) {
  for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
    $arg = $CommandArgs[$index]
    if (($arg -eq "--model" -or $arg -eq "-m") -and $index + 1 -lt $CommandArgs.Count) { return $CommandArgs[$index + 1] }
    if ($arg.StartsWith("--model=", [StringComparison]::Ordinal)) { return $arg.Substring("--model=".Length) }
  }
  return ""
}

function Extract-Resume-Target([string[]]$CommandArgs) {
  if ($Tool -eq 'kiro-cli') {
    $target = ''
    $valueOptions = @('--agent','--model','--effort','--trust-tools','--format','-f','--output-format','--wrap','-w','--agent-engine','--mode','--repo','--session-source','--delete-session','-d')
    for ($index=0; $index -lt $CommandArgs.Count; $index++) {
      $arg=$CommandArgs[$index]
      if ($arg -eq '--') { break }
      if ($arg -eq '--resume-id') {
        if ($index+1 -lt $CommandArgs.Count) { $target=$CommandArgs[++$index] }
      } elseif ($arg.StartsWith('--resume-id=',[StringComparison]::Ordinal)) {
        $target=$arg.Substring('--resume-id='.Length)
      } elseif ($valueOptions -contains $arg) { $index++ }
    }
    return $target
  }
  if ($Tool -eq 'agy') {
    $target = ''
    $valueOptions = @('--agent','--model','--mode','--effort','--project','--log-file','--add-dir','--json-schema','--input-format','--output-format','--print-timeout','--print','-p','--prompt','--prompt-interactive','-i')
    for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
      $arg = $CommandArgs[$index]
      if ($arg -eq '--') { break }
      if ($arg -eq '--conversation') {
        if ($index + 1 -lt $CommandArgs.Count) { $target = $CommandArgs[++$index] }
      } elseif ($arg.StartsWith('--conversation=', [StringComparison]::Ordinal)) {
        $target = $arg.Substring('--conversation='.Length)
      } elseif ($valueOptions -contains $arg) {
        $index++
      }
    }
    return $target
  }
  for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
    $arg = $CommandArgs[$index]
    if (($arg -eq "--resume" -or
          $arg -eq "-r" -or
          $arg -eq "--resume-id" -or
          $arg -eq "--session" -or
          $arg -eq "--session-id") -and
        $index + 1 -lt $CommandArgs.Count -and
        -not $CommandArgs[$index + 1].StartsWith("-", [StringComparison]::Ordinal)) {
      return $CommandArgs[$index + 1]
    }
    if ($arg -eq "resume" -and
        $index + 1 -lt $CommandArgs.Count -and
        -not $CommandArgs[$index + 1].StartsWith("-", [StringComparison]::Ordinal)) {
      return $CommandArgs[$index + 1]
    }
    if ($arg.StartsWith("--resume=", [StringComparison]::Ordinal)) { return $arg.Substring("--resume=".Length) }
    if ($arg.StartsWith("--resume-id=", [StringComparison]::Ordinal)) { return $arg.Substring("--resume-id=".Length) }
    if ($arg.StartsWith("--session=", [StringComparison]::Ordinal)) { return $arg.Substring("--session=".Length) }
    if ($arg.StartsWith("--session-id=", [StringComparison]::Ordinal)) { return $arg.Substring("--session-id=".Length) }
  }
  return ""
}

function Extract-Omp-Session-Directory([string[]]$CommandArgs) {
  $value = ""
  $launchDirectory = (Get-Location).Path
  $cwdValue = ""
  for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
    $arg = $CommandArgs[$index]
    if ($arg -eq "--cwd" -and $index + 1 -lt $CommandArgs.Count) {
      $cwdValue = $CommandArgs[$index + 1]
    }
    if ($arg.StartsWith("--cwd=", [StringComparison]::Ordinal)) {
      $cwdValue = $arg.Substring("--cwd=".Length)
    }
    if ($arg -eq "--session-dir" -and $index + 1 -lt $CommandArgs.Count) {
      $value = $CommandArgs[$index + 1]
    }
    if ($arg.StartsWith("--session-dir=", [StringComparison]::Ordinal)) {
      $value = $arg.Substring("--session-dir=".Length)
    }
  }
  if (-not [string]::IsNullOrWhiteSpace($cwdValue)) {
    $launchDirectory = if ([IO.Path]::IsPathRooted($cwdValue)) { $cwdValue } else { Join-Path $launchDirectory $cwdValue }
  }
  if ([string]::IsNullOrWhiteSpace($value)) { return "" }
  try {
    if ([IO.Path]::IsPathRooted($value)) {
      return [IO.Path]::GetFullPath($value)
    }
    return [IO.Path]::GetFullPath((Join-Path $launchDirectory $value))
  } catch {
    return $value
  }
}

function Is-Metadata-Invocation([string[]]$CommandArgs) {
  if ($CommandArgs.Count -eq 0) { return $false }
  foreach ($arg in $CommandArgs) {
    switch -Regex ($arg) {
      '^(--version|-V|version)$' { return $true }
      '^(--help|-h|help)$' { return $true }
      '^(features|--features)$' { return $true }
      '^(auth|login|logout|doctor|update|upgrade|config)$' { return $true }
    }
  }
  return $false
}

function Is-Passthrough-Invocation([string[]]$CommandArgs) {
  if ($CommandArgs.Count -eq 0) { return $false }
  if ($Tool -eq 'kiro-cli') {
    $valueOptions = @('--agent','--model','--effort','--trust-tools','--resume-id','--format','-f','--output-format','--wrap','-w','--agent-engine','--mode','--repo','--session-source')
    $commands = @('debug','settings','setup','update','diagnostic','init','theme','issue','login','logout','whoami','profile','user','doctor','launch','quit','restart','integrations','translate','dashboard','crew','mcp','powers','inline','agent','acp','help')
    $chat = $false
    for ($index=0; $index -lt $CommandArgs.Count; $index++) {
      $arg=$CommandArgs[$index]
      if ($arg -eq '--') { return $false }
      if ($arg -in @('--help','-h','--help-all','--version','-V','--list-models','--list-sessions','-l','--sessions','--delete-session','-d') -or $arg.StartsWith('--delete-session=')) { return $true }
      if ($valueOptions -contains $arg) { $index++; continue }
      if ($arg.StartsWith('-')) { continue }
      if (-not $chat -and $arg -eq 'chat') { $chat=$true; continue }
      if ($chat) { return $false }
      return $commands -contains $arg
    }
    return $false
  }
  if ($Tool -eq 'kimi' -or $Tool -eq 'kimi-code') {
    $valueOptions = @('--agent','--agent-file','--model','-m','--prompt','-p','--config','--config-file','--work-dir','-w','--skills-dir','--add-dir','--output-format')
    $sessionOptions = @('--session','-S','--resume','-r')
    $commands = @('provider','session','export','fork','login','logout','doctor','vis','install-desktop','app','migrate','upgrade','update','acp','web','server','rc','remote','term','mcp','help','config','info')
    for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
      $arg = $CommandArgs[$index]
      if ($arg -eq '--') { return $false }
      if ($arg -in @('--help','-h','--version','-V')) { return $true }
      if ($arg -cin $sessionOptions) {
        if ($index + 1 -lt $CommandArgs.Count -and -not $CommandArgs[$index + 1].StartsWith('-')) { $index++ }
        continue
      }
      if ($arg -cin $valueOptions) {
        if ($index + 1 -ge $CommandArgs.Count) { return $false }
        $index++
        continue
      }
      if ($arg.StartsWith('-')) { continue }
      return $arg -cin $commands
    }
    return $false
  }
  if ($Tool -eq 'agy') {
    $valueOptions = @('--agent','--model','--mode','--effort','--project','--conversation','--log-file','--add-dir','--json-schema','--input-format','--output-format','--print-timeout')
    $booleanOptions = @('--continue','-c','--new-project','--remote-control','--sandbox','--dangerously-skip-permissions','--disable-slash-commands')
    $commands = @('agent','agents','models','changelog','plugins','plugin','mcp','install','remote-control','mic-serve','update','help')
    for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
      $arg = $CommandArgs[$index]
      if ($arg -eq '--') { return $false }
      if ($arg -in @('--help','-h','--version','-V')) { return $true }
      # A prompt value is text, even when it matches a subcommand or flag.
      if ($arg -match '^(--print|--prompt|--prompt-interactive|-p|-i)(=|$)') { return $false }
      if ($valueOptions -contains $arg) {
        if ($index + 1 -ge $CommandArgs.Count) { return $false }
        $index++
        continue
      }
      if ($booleanOptions -contains $arg) { continue }
      $name = ($arg -split '=',2)[0]
      if ($arg.Contains('=') -and (($valueOptions -contains $name) -or ($booleanOptions -contains $name))) { continue }
      return ($commands -contains $arg)
    }
    return $false
  }
  if ($Tool -eq "omp") {
    $command = ""
    for ($index = 0; $index -lt $CommandArgs.Count; $index++) {
      $arg = $CommandArgs[$index]
      if ($arg -eq "--") { return $false }
      if ($arg -eq "--alias" -or $arg.StartsWith("--alias=", [StringComparison]::Ordinal)) {
        return $true
      }
      if ($arg -match '^(--help|-h|--version|-v)$') { return $true }
      if ($arg -match '^(--cwd|--config|--mode|--fork|--provider|--model|--smol|--slow|--prewalk-into|--plan-yolo-into|--max-time|--api-key|--system-prompt|--append-system-prompt|--provider-session-id|--prompt-cache-key|--session-dir|--models|--tools|--thinking|--export|--hook|--extension|-e|--plugin-dir|--skills|--approval-mode|--profile)$') {
        if ($index + 1 -ge $CommandArgs.Count) { return $true }
        $index++
        continue
      }
      if ($arg -eq "--plan") {
        if ($index + 1 -lt $CommandArgs.Count -and
            -not $CommandArgs[$index + 1].StartsWith("-")) {
          $index++
        }
        continue
      }
      if ($arg -match '^(--resume|-r|--session)$') {
        if ($index + 1 -lt $CommandArgs.Count -and
            -not $CommandArgs[$index + 1].StartsWith("-") -and
            -not [string]::IsNullOrEmpty($CommandArgs[$index + 1])) {
          $index++
        }
        continue
      }
      if ($arg -match '^(--allow-home|--continue|-c|--no-session|--no-tools|--no-lsp|--no-pty|--hide-thinking|--advisor|--prewalk|--no-prewalk|--plan-yolo|--print|-p|--print-thoughts|--no-extensions|--no-skills|--no-rules|--no-title|--auto-approve|--yolo)$') {
        continue
      }
      if ($arg.StartsWith("--") -and $arg.Contains("=")) {
        continue
      }
      if ($arg.StartsWith("--")) {
        if ($index + 1 -lt $CommandArgs.Count -and -not $CommandArgs[$index + 1].StartsWith("-")) {
          $index++
        }
        continue
      }
      if ($arg.StartsWith("-")) {
        continue
      }
      $command = $arg
      break
    }
    if ($command -match '^(acp|agents|auth-broker|auth-gateway|bench|commit|completions|__complete|config|dry-balance|gallery|gc|grep|grievances|install|join|models|plugin|read|say|search|q|setup|shell|ssh|stats|tiny-models|token|ttsr|update|usage|worktree|wt)$') {
      return $true
    }
  }
  switch -Regex ($CommandArgs[0]) {
    '^(--help|-h|help|--version|-V|-v|version|features|--features|auth|login|logout|doctor|update|upgrade|config|info|export|mcp|plugin|vis|web|term|acp|app-server|__background-task-worker|__web-worker)$' { return $true }
  }
  return $false
}

function Get-Memory-Prompt-File {
  $promptFile = $env:DMUX_AI_MEMORY_PROMPT_FILE
  if ([string]::IsNullOrWhiteSpace($promptFile) -or -not (Test-Path -LiteralPath $promptFile)) {
    return ""
  }
  return $promptFile
}

function Invoke-Managed-Instructions([string]$Kind, [string]$PromptFile, [string[]]$CommandArgs) {
  $helperPath = Join-Path $wrapperDir 'codux-wrapper-helper.exe'
  if (-not (Test-Path -LiteralPath $helperPath)) { throw 'Codux instruction helper is missing' }
  $startInfo = New-Object System.Diagnostics.ProcessStartInfo
  $startInfo.FileName = $helperPath
  $startInfo.Arguments = '--codux-wrapper-helper managed-instructions --json-stdin'
  $startInfo.UseShellExecute = $false
  $startInfo.CreateNoWindow = $true
  $startInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
  $startInfo.RedirectStandardInput = $true
  $startInfo.RedirectStandardOutput = $true
  $startInfo.RedirectStandardError = $true
  $startInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
  $child = [Diagnostics.Process]::Start($startInfo)
  try {
    $request = @{kind=$Kind;contextFile=$PromptFile;args=@($CommandArgs);binary=$realBin} | ConvertTo-Json -Depth 8 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($request)
    $child.StandardInput.BaseStream.Write($bytes,0,$bytes.Length)
    $child.StandardInput.Close()
    $response = $child.StandardOutput.ReadToEnd()
    $failure = $child.StandardError.ReadToEnd()
    $child.WaitForExit()
    if ($child.ExitCode -ne 0) { throw "Codux instruction helper failed: $failure" }
    foreach ($argument in (ConvertFrom-Json -InputObject $response)) { [string]$argument }
  } finally { $child.Dispose() }
}

function Apply-Append-System-Prompt([string[]]$CommandArgs, [string]$Strategy, [string]$Label) {
  if ($memoryInjectionStrategy -ne $Strategy) { return $CommandArgs }
  if (Has-Option-Value $CommandArgs @("--append-system-prompt")) {
    $promptFile = Get-Memory-Prompt-File
    if ([string]::IsNullOrWhiteSpace($promptFile)) { return $CommandArgs }
    return Invoke-Managed-Instructions 'append' $promptFile $CommandArgs
  }
  $promptFile = Get-Memory-Prompt-File
  if ([string]::IsNullOrWhiteSpace($promptFile)) {
    Write-Live-Log "$Label instructions skipped: prompt file missing"
    return $CommandArgs
  }
  try {
    $prompt = Get-Content -LiteralPath $promptFile -Raw -Encoding UTF8
    if ([string]::IsNullOrWhiteSpace($prompt)) {
      Write-Live-Log "$Label instructions skipped: prompt empty path=$promptFile"
      return $CommandArgs
    }
    Write-Live-Log "$Label instructions injected path=$promptFile chars=$($prompt.Length)"
    return @("--append-system-prompt", $prompt) + $CommandArgs
  } catch {
    Write-Live-Log "$Label instructions skipped: failed to read prompt path=$promptFile error=$($_.Exception.Message)"
    return $CommandArgs
  }
}

function Apply-Managed-CodeWhale-Context([string[]]$CommandArgs) {
  if ($Tool -ne 'codewhale') { return }
  $promptFile = Get-Memory-Prompt-File
  if ([string]::IsNullOrWhiteSpace($promptFile)) { return }
  $paths = @(Invoke-Managed-Instructions 'codewhale' $promptFile $CommandArgs)
  if ($paths.Count -ne 1 -or -not (Test-Path -LiteralPath $paths[0])) {
    throw 'Codux: unable to prepare CodeWhale project instructions.'
  }
  $env:DEEPSEEK_MANAGED_CONFIG_PATH = $paths[0]
}

function Apply-Managed-Kimi-Context([string[]]$CommandArgs) {
  if ($memoryInjectionStrategy -ne 'kimiAgent') { return $CommandArgs }
  $promptFile = $env:CODUX_BLACKBOARD_CONTEXT_FILE
  if ([string]::IsNullOrWhiteSpace($promptFile)) {
    $promptFile = Get-Memory-Prompt-File
  } elseif (-not (Test-Path -LiteralPath $promptFile)) {
    [Console]::Error.WriteLine('Codux: Kimi project instructions are missing.')
    exit 1
  }
  if ([string]::IsNullOrWhiteSpace($promptFile)) { return $CommandArgs }
  try {
    $env:CODUX_NATIVE_KIMI_CONTEXT_MANIFEST = $null
    $prepared = @(Invoke-Managed-Instructions 'kimi-launch' $promptFile $CommandArgs)
    if ($prepared.Count -lt 2) { throw 'Invalid Kimi launch response.' }
    if ($prepared[0] -eq 'native') {
      if (-not (Test-Path -LiteralPath $prepared[1])) { throw 'Kimi Code instruction manifest is missing.' }
      $env:CODUX_NATIVE_KIMI_CONTEXT_MANIFEST = $prepared[1]
      $env:CODUX_MANAGED_INSTRUCTIONS_HELPER = Join-Path $wrapperDir 'codux-wrapper-helper.exe'
    } elseif ($prepared[0] -ne 'python') { throw 'Invalid Kimi instruction strategy.' }
    for ($index = 2; $index -lt $prepared.Count; $index++) { [string]$prepared[$index] }
    return
  } catch {
    [Console]::Error.WriteLine("Codux: unable to load Kimi project instructions. $($_.Exception.Message)")
    exit 1
  }
}

function Apply-Managed-Agy-Context([string[]]$CommandArgs) {
  if ($Tool -ne 'agy' -or $memoryInjectionStrategy -ne 'agyHook') { return }
  try {
    $env:CODUX_AGY_CONTEXT_MANIFEST = $null
    $env:CODUX_MANAGED_INSTRUCTIONS_HELPER = $null
    $promptFile = $env:CODUX_BLACKBOARD_CONTEXT_FILE
    if ([string]::IsNullOrWhiteSpace($promptFile)) {
      $promptFile = Get-Memory-Prompt-File
    } elseif (-not (Test-Path -LiteralPath $promptFile)) {
      throw 'Agy project instructions are missing.'
    }
    if ([string]::IsNullOrWhiteSpace($promptFile)) { return }
    $paths = @(Invoke-Managed-Instructions 'agy' $promptFile $CommandArgs)
    if ($paths.Count -ne 1 -or -not (Test-Path -LiteralPath $paths[0])) {
      throw 'Unable to prepare Agy project instructions.'
    }
    $env:CODUX_AGY_CONTEXT_MANIFEST = $paths[0]
    $env:CODUX_MANAGED_INSTRUCTIONS_HELPER = Join-Path $wrapperDir 'codux-wrapper-helper.exe'
  } catch {
    [Console]::Error.WriteLine("Codux: unable to load Agy project instructions. $($_.Exception.Message)")
    exit 1
  }
}

function Apply-Managed-Kiro-Context([string[]]$CommandArgs) {
  if ($memoryInjectionStrategy -ne 'kiroHook') { return }
  try {
    $env:CODUX_KIRO_CONTEXT_MANIFEST=$null
    $env:CODUX_KIRO_CONTEXT_MODE=$null
    $promptFile=$env:CODUX_BLACKBOARD_CONTEXT_FILE
    if ([string]::IsNullOrWhiteSpace($promptFile)) { $promptFile=Get-Memory-Prompt-File }
    elseif (-not (Test-Path -LiteralPath $promptFile)) { throw 'Kiro project instructions are missing.' }
    if ([string]::IsNullOrWhiteSpace($promptFile)) { return }
    $prepared=@(Invoke-Managed-Instructions 'kiro' $promptFile $CommandArgs)
    if ($prepared.Count -ne ($CommandArgs.Count+2) -or -not (Test-Path -LiteralPath $prepared[0]) -or $prepared[1] -notin @('inline','global')) { throw 'Invalid Kiro instruction preparation.' }
    $env:CODUX_KIRO_CONTEXT_MANIFEST=$prepared[0]
    $env:CODUX_KIRO_CONTEXT_MODE=$prepared[1]
    $env:CODUX_MANAGED_INSTRUCTIONS_HELPER=Join-Path $wrapperDir 'codux-wrapper-helper.exe'
  } catch {
    [Console]::Error.WriteLine("Codux: unable to load Kiro project instructions. $($_.Exception.Message)")
    exit 1
  }
}

function Codex-Hooks-Feature-Flag([string]$Binary, [string]$SearchPath) {
  $previousPath = $env:PATH
  try {
    $env:PATH = $SearchPath
    $features = & $Binary features list 2>$null
    if ($features -match "(?m)^hooks\\s") { return "hooks" }
    if ($features -match "(?m)^codex_hooks\\s") { return "codex_hooks" }
  } finally {
    $env:PATH = $previousPath
  }
  return "hooks"
}

function Invoke-Real-Binary([string]$Binary, [string[]]$CommandArgs, [string]$SearchPath, [string]$LaunchDir) {
  $previousPath = $env:PATH
  try {
    $env:PATH = $SearchPath
    $nativeHelper = Join-Path $wrapperDir 'codux-wrapper-helper.exe'
    if (($Binary.EndsWith('.exe', [StringComparison]::OrdinalIgnoreCase) -or $Tool -eq 'codex' -or (($Tool -eq 'kimi' -or $Tool -eq 'kimi-code') -and -not [string]::IsNullOrWhiteSpace($env:CODUX_NATIVE_KIMI_CONTEXT_MANIFEST))) -and (Test-Path -LiteralPath $nativeHelper)) {
      $requestPath = Join-Path ([IO.Path]::GetTempPath()) "codux-launch-$([Guid]::NewGuid().ToString('N')).json"
      try {
        $cwd = if (-not [string]::IsNullOrWhiteSpace($LaunchDir) -and (Test-Path -LiteralPath $LaunchDir)) { $LaunchDir } else { (Get-Location).ProviderPath }
        $request = @{binary=$Binary;args=@($CommandArgs);cwd=$cwd;tool=$Tool} | ConvertTo-Json -Depth 5 -Compress
        [IO.File]::WriteAllText($requestPath, $request, [Text.UTF8Encoding]::new($false))
        & $nativeHelper --codux-wrapper-helper native-launch $requestPath
        $script:DMUX_WRAPPER_EXIT_CODE = $LASTEXITCODE
        return
      } finally {
        if (Test-Path -LiteralPath $requestPath) { Remove-Item -LiteralPath $requestPath -Force }
      }
    }
    if (-not [string]::IsNullOrWhiteSpace($LaunchDir) -and (Test-Path -LiteralPath $LaunchDir)) {
      Push-Location -LiteralPath $LaunchDir
      try {
        & $Binary @CommandArgs
        $script:DMUX_WRAPPER_EXIT_CODE = $LASTEXITCODE
        return
      } finally {
        Pop-Location
      }
    }
    & $Binary @CommandArgs
    $script:DMUX_WRAPPER_EXIT_CODE = $LASTEXITCODE
  } finally {
    $env:PATH = $previousPath
  }
}

function Emit-Wrapper-SessionEnd {
  if ([string]::IsNullOrWhiteSpace($env:DMUX_SESSION_ID) -or
      [string]::IsNullOrWhiteSpace($env:DMUX_RUNTIME_EVENT_DIR)) {
    return
  }
  $helper = Join-Path $wrapperDir "dmux-ai-state.ps1"
  if (-not (Test-Path -LiteralPath $helper)) { return }
  try {
    & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File $helper "session-end" "$env:DMUX_RUNTIME_OWNER" "$Tool" *> $null
  } catch {
  }
}

function Write-Runtime-Binding([string]$ExternalSessionId, [string]$Model, [string]$TranscriptPath, [string]$SessionOrigin) {
  if ([string]::IsNullOrWhiteSpace($env:DMUX_AI_RUNTIME_BINDING_DIR) -or
      [string]::IsNullOrWhiteSpace($env:DMUX_SESSION_ID) -or
      [string]::IsNullOrWhiteSpace($env:DMUX_PROJECT_ID) -or
      [string]::IsNullOrWhiteSpace($Tool)) {
    return
  }
  try {
    New-Item -ItemType Directory -Force -Path $env:DMUX_AI_RUNTIME_BINDING_DIR | Out-Null
    $bindingIdSeed = if ([string]::IsNullOrWhiteSpace($env:DMUX_SESSION_INSTANCE_ID)) { $env:DMUX_SESSION_ID } else { $env:DMUX_SESSION_INSTANCE_ID }
    $payload = [ordered]@{
      runtimeBindingId = "$bindingIdSeed-$Tool"
      terminalId = $env:DMUX_SESSION_ID
      terminalInstanceId = if ([string]::IsNullOrWhiteSpace($env:DMUX_SESSION_INSTANCE_ID)) { $null } else { $env:DMUX_SESSION_INSTANCE_ID }
      tool = $Tool
      projectId = $env:DMUX_PROJECT_ID
      projectName = if ([string]::IsNullOrWhiteSpace($env:DMUX_PROJECT_NAME)) { "Workspace" } else { $env:DMUX_PROJECT_NAME }
      projectPath = if ([string]::IsNullOrWhiteSpace($env:DMUX_PROJECT_PATH)) { $null } else { $env:DMUX_PROJECT_PATH }
      sessionTitle = if ([string]::IsNullOrWhiteSpace($env:DMUX_SESSION_TITLE)) { "Terminal" } else { $env:DMUX_SESSION_TITLE }
      launchStartedAt = ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0)
      externalSessionId = if ([string]::IsNullOrWhiteSpace($ExternalSessionId)) { $null } else { $ExternalSessionId }
      transcriptPath = if ([string]::IsNullOrWhiteSpace($TranscriptPath)) { $null } else { $TranscriptPath }
      model = if ([string]::IsNullOrWhiteSpace($Model)) { $null } else { $Model }
      sessionOrigin = if ([string]::IsNullOrWhiteSpace($SessionOrigin)) { $null } else { $SessionOrigin }
      updatedAt = ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0)
    }
    $path = Join-Path $env:DMUX_AI_RUNTIME_BINDING_DIR "$($env:DMUX_SESSION_ID)-$Tool.json"
    $tmp = "$path.tmp"
    $json = $payload | ConvertTo-Json -Depth 8 -Compress
    $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($tmp, $json, $utf8NoBom)
    Move-Item -Force -LiteralPath $tmp -Destination $path
  } catch {
    Write-Live-Log "failed to write runtime binding: $($_.Exception.Message)"
  }
}

function Convert-Osc-ColorRef([string]$Payload) {
  # Payload format from the app palette: rgb:rrrr/gggg/bbbb (each byte doubled).
  if ($Payload -notmatch '^rgb:([0-9a-f]{2})[0-9a-f]{2}/([0-9a-f]{2})[0-9a-f]{2}/([0-9a-f]{2})[0-9a-f]{2}$') { return $null }
  $r = [Convert]::ToUInt32($Matches[1], 16)
  $g = [Convert]::ToUInt32($Matches[2], 16)
  $b = [Convert]::ToUInt32($Matches[3], 16)
  return [uint32](($b -shl 16) -bor ($g -shl 8) -bor $r)
}

function Seed-Console-Color-Table([uint32]$Foreground, [uint32]$Background) {
  # TUIs that get no OSC reply fall back to GetConsoleScreenBufferInfoEx, which
  # reads conhost's legacy 16-color table: rewrite the two entries behind the
  # default attributes so that fallback also reports the app theme colors.
  try {
    Add-Type -Namespace CoduxWrapper -Name ConsoleColorSeed -ErrorAction Stop -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)]
public struct Coord { public short X; public short Y; }
[StructLayout(LayoutKind.Sequential)]
public struct SmallRect { public short Left; public short Top; public short Right; public short Bottom; }
[StructLayout(LayoutKind.Sequential)]
public struct BufferInfoEx {
  public uint cbSize;
  public Coord dwSize;
  public Coord dwCursorPosition;
  public ushort wAttributes;
  public SmallRect srWindow;
  public Coord dwMaximumWindowSize;
  public ushort wPopupAttributes;
  public int bFullscreenSupported;
  [MarshalAs(UnmanagedType.ByValArray, SizeConst = 16)]
  public uint[] ColorTable;
}
[DllImport("kernel32.dll", SetLastError = true)]
private static extern IntPtr GetStdHandle(int nStdHandle);
[DllImport("kernel32.dll", SetLastError = true)]
private static extern bool GetConsoleScreenBufferInfoEx(IntPtr handle, ref BufferInfoEx info);
[DllImport("kernel32.dll", SetLastError = true)]
private static extern bool SetConsoleScreenBufferInfoEx(IntPtr handle, ref BufferInfoEx info);
public static bool Seed(uint foreground, uint background) {
  IntPtr handle = GetStdHandle(-11);
  if (handle == IntPtr.Zero || handle == new IntPtr(-1)) { return false; }
  BufferInfoEx info = new BufferInfoEx();
  info.ColorTable = new uint[16];
  info.cbSize = (uint)Marshal.SizeOf(typeof(BufferInfoEx));
  if (!GetConsoleScreenBufferInfoEx(handle, ref info)) { return false; }
  info.ColorTable[info.wAttributes & 0x0f] = foreground;
  info.ColorTable[(info.wAttributes >> 4) & 0x0f] = background;
  info.srWindow.Bottom++; // Set treats the window rect as exclusive.
  return SetConsoleScreenBufferInfoEx(handle, ref info);
}
'@
    if (-not [CoduxWrapper.ConsoleColorSeed]::Seed($Foreground, $Background)) {
      Write-Live-Log "console color table seed failed"
    }
  } catch {
    Write-Live-Log "console color table seed unavailable: $($_.Exception.Message)"
  }
}

$searchPath = Filter-Wrapper-Path $env:PATH
if ([string]::IsNullOrWhiteSpace($searchPath)) {
  $searchPath = Filter-Wrapper-Path $env:DMUX_ORIGINAL_PATH
}
$runtimePath = Join-PathList @($wrapperBin, $searchPath)

$realBin = Find-Real-Binary $Tool $searchPath
if ([string]::IsNullOrWhiteSpace($realBin)) {
  Write-Live-Log "launch failed tool=$Tool reason=missing-binary"
  [Console]::Error.WriteLine("$Tool is not installed or not available in PATH.")
  exit 127
}

# Seed the console's reported default colors (OSC 10/11 set) with the app
# theme: ConPTY answers color queries itself from its own black palette, so
# TUIs would detect a dark background under a light theme.
if (-not [Console]::IsOutputRedirected) {
  $esc = [char]27
  if (-not [string]::IsNullOrWhiteSpace($env:DMUX_TERMINAL_OSC_FG)) {
    [Console]::Out.Write("$esc]10;$($env:DMUX_TERMINAL_OSC_FG)$esc\")
  }
  if (-not [string]::IsNullOrWhiteSpace($env:DMUX_TERMINAL_OSC_BG)) {
    [Console]::Out.Write("$esc]11;$($env:DMUX_TERMINAL_OSC_BG)$esc\")
  }
  $oscFgRef = Convert-Osc-ColorRef ([string]$env:DMUX_TERMINAL_OSC_FG)
  $oscBgRef = Convert-Osc-ColorRef ([string]$env:DMUX_TERMINAL_OSC_BG)
  if ($null -ne $oscFgRef -and $null -ne $oscBgRef) {
    Seed-Console-Color-Table $oscFgRef $oscBgRef
  }
}

if (Is-Passthrough-Invocation $ToolArgs) {
  Invoke-Real-Binary $realBin $ToolArgs $runtimePath ""
  $exitCode = if ($null -eq $script:DMUX_WRAPPER_EXIT_CODE) { 0 } else { $script:DMUX_WRAPPER_EXIT_CODE }
  exit $exitCode
}

$settings = Read-Tool-Settings
$memoryInjectionStrategy = Tool-Memory-Injection-Strategy $Tool
$permissionKey = Tool-Config-Key $Tool
$modelKey = Tool-Model-Key $Tool
$permissionMode = if ($settings -and $permissionKey) {
  [string]$settings.$permissionKey
} else {
  ""
}
$configuredModel = if ($settings -and $modelKey) { [string]$settings.$modelKey } else { "" }
$codexEffort = if ($settings) { [string]$settings.codexEffort } else { "" }
if ($codexEffort -notin @("minimal", "low", "medium", "high", "xhigh")) {
  $codexEffort = ""
}

$launchArgs = @($ToolArgs)
if ($Tool -ne "kiro-cli" -and -not [string]::IsNullOrWhiteSpace($configuredModel) -and -not (Has-Option-Value $launchArgs @("--model", "-m"))) {
  if ($Tool -eq "codex") {
    $launchArgs = @("--model=$configuredModel") + $launchArgs
  } else {
    $launchArgs = @("--model", $configuredModel) + $launchArgs
  }
}

if ($Tool -eq "codex" -and -not [string]::IsNullOrWhiteSpace($codexEffort) -and -not (Has-Config-Key $launchArgs "model_reasoning_effort")) {
  $launchArgs = @("-c", "model_reasoning_effort=`"$codexEffort`"") + $launchArgs
}

if ($memoryInjectionStrategy -eq "codexDeveloperInstructions" -and
    $permissionMode -ne "fullAccess" -and
    -not [string]::IsNullOrWhiteSpace($env:DMUX_AI_MEMORY_WORKSPACE_ROOT) -and
    (Test-Path -LiteralPath $env:DMUX_AI_MEMORY_WORKSPACE_ROOT) -and
    -not [string]::IsNullOrWhiteSpace($env:DMUX_PROJECT_PATH) -and
    (Test-Path -LiteralPath $env:DMUX_PROJECT_PATH) -and
    -not (Codex-Has-Sandbox-Mode-Arg $launchArgs)) {
  $launchArgs = @("--sandbox", "workspace-write") + $launchArgs
}

if ($memoryInjectionStrategy -eq "codexDeveloperInstructions" -and
    -not [string]::IsNullOrWhiteSpace($env:DMUX_AI_MEMORY_WORKSPACE_ROOT) -and
    (Test-Path -LiteralPath $env:DMUX_AI_MEMORY_WORKSPACE_ROOT) -and
    -not [string]::IsNullOrWhiteSpace($env:DMUX_PROJECT_PATH) -and
    (Test-Path -LiteralPath $env:DMUX_PROJECT_PATH) -and
    ($permissionMode -eq "fullAccess" -or (Codex-Allows-Additional-Writable-Roots $launchArgs)) -and
    -not (Has-Option-Value $launchArgs @("-C", "--cd"))) {
  $launchArgs = @("-C", $env:DMUX_PROJECT_PATH, "--add-dir", $env:DMUX_AI_MEMORY_WORKSPACE_ROOT) + $launchArgs
}

if ($memoryInjectionStrategy -eq "codexDeveloperInstructions" -and
    ((-not [string]::IsNullOrWhiteSpace($env:CODUX_BLACKBOARD_CONTEXT_FILE) -and (Test-Path -LiteralPath $env:CODUX_BLACKBOARD_CONTEXT_FILE)) -or
     (-not [string]::IsNullOrWhiteSpace($env:DMUX_AI_MEMORY_WORKSPACE_ROOT) -and (Test-Path -LiteralPath $env:DMUX_AI_MEMORY_WORKSPACE_ROOT)))) {
  $memoryAgents = if (-not [string]::IsNullOrWhiteSpace($env:CODUX_BLACKBOARD_CONTEXT_FILE)) { $env:CODUX_BLACKBOARD_CONTEXT_FILE } else { Join-Path $env:DMUX_AI_MEMORY_WORKSPACE_ROOT "AGENTS.md" }
  if (Test-Path -LiteralPath $memoryAgents) {
    try {
      $merged = @(Invoke-Managed-Instructions 'codex' $memoryAgents $launchArgs)
      $content = $merged[1].Substring('developer_instructions='.Length) | ConvertFrom-Json
      if (-not [string]::IsNullOrWhiteSpace($content)) {
        # CLI precedence is required: trusted project settings override profiles.
        $launchArgs = $merged
        Write-Live-Log "codex instructions injected path=$memoryAgents chars=$($content.Length)"
      } else {
        Write-Live-Log "codex instructions skipped: AGENTS.md empty path=$memoryAgents"
      }
    } catch {
      Write-Live-Log "codex instructions skipped: failed to read AGENTS.md path=$memoryAgents error=$($_.Exception.Message)"
      if (-not [string]::IsNullOrWhiteSpace($env:CODUX_BLACKBOARD_CONTEXT_FILE)) {
        [Console]::Error.WriteLine('Codux: unable to load project instructions.')
        exit 1
      }
    }
  } else {
    Write-Live-Log "codex instructions skipped: AGENTS.md missing path=$memoryAgents"
  }
} elseif ($memoryInjectionStrategy -eq "codexDeveloperInstructions") {
  if (Has-Config-Key $launchArgs "developer_instructions") {
    Write-Live-Log "codex instructions skipped: developer_instructions already provided"
  } else {
    Write-Live-Log "codex instructions skipped: memory workspace missing"
  }
}

if ($Tool -eq "codex" -and -not (Is-Metadata-Invocation $launchArgs) -and -not (Has-Arg $launchArgs "--enable")) {
  $hooksFeature = Codex-Hooks-Feature-Flag $realBin $searchPath
  $launchArgs = @("--enable", $hooksFeature) + $launchArgs
} else {
  $hooksFeature = ""
}

if ($permissionMode -eq "fullAccess") {
  if ($Tool -eq "codex") {
    if (-not (Has-Arg $launchArgs "--dangerously-bypass-approvals-and-sandbox") -and
        -not (Has-Arg $launchArgs "--full-auto") -and
        -not (Has-Option-Value $launchArgs @("--sandbox", "--ask-for-approval", "-s", "-a"))) {
      $launchArgs = @("--dangerously-bypass-approvals-and-sandbox") + $launchArgs
    }
  } elseif ($Tool -eq "claude" -or $Tool -eq "claude-code" -or $Tool -eq "reclaude") {
    if (-not (Has-Arg $launchArgs "--dangerously-skip-permissions") -and
        -not (Has-Arg $launchArgs "--allow-dangerously-skip-permissions") -and
        -not (Has-Option-Value $launchArgs @("--permission-mode"))) {
      $launchArgs = @("--dangerously-skip-permissions") + $launchArgs
    }
  } elseif ($Tool -eq "agy") {
    if (-not (Has-Option-Value $launchArgs @("--approval-mode", "--mode", "--sandbox", "--dangerously-skip-permissions")) -and
        -not (Has-Arg $launchArgs "--yolo") -and
        -not (Has-Arg $launchArgs "-y")) {
      $launchArgs = @("--dangerously-skip-permissions") + $launchArgs
    }
  } elseif ($Tool -eq "omp") {
    if (-not (Has-Option-Value $launchArgs @("--approval-mode")) -and
        -not (Has-Arg $launchArgs "--auto-approve") -and
        -not (Has-Arg $launchArgs "--yolo")) {
      $launchArgs = @("--approval-mode", "yolo") + $launchArgs
    }
  } elseif ($Tool -eq "kimi" -or $Tool -eq "kimi-code") {
    # Kimi Code uses the generic model flag, but its permission flags differ from agy.
  } elseif ($Tool -eq "opencode" -or $Tool -eq "mimo") {
    if (-not (Has-Arg $launchArgs "--dangerously-skip-permissions")) {
      $launchArgs = @("--dangerously-skip-permissions") + $launchArgs
    }
  } elseif ($Tool -eq "codewhale") {
    if (-not (Has-Arg $launchArgs "--yolo")) {
      $launchArgs = @("--yolo") + $launchArgs
    }
  }
}

# Covers both the injected flag (fullAccess) and a user-supplied one: when
# elevated, Claude Code refuses the bypass and prompts will still appear.
if (($Tool -eq "claude" -or $Tool -eq "claude-code" -or $Tool -eq "reclaude") -and
    (Has-Arg $launchArgs "--dangerously-skip-permissions") -and
    (Test-Elevated-Process)) {
  [Console]::Error.WriteLine("Codux: this terminal is running as Administrator; Claude Code refuses --dangerously-skip-permissions when elevated, so permission prompts will still appear.")
  Write-Live-Log "elevation warning emitted tool=$Tool (bypass flag ineffective under admin)"
}

if ($memoryInjectionStrategy -eq "appendSystemPrompt") {
  $launchArgs = Apply-Append-System-Prompt $launchArgs "appendSystemPrompt" $Tool
}
$launchArgs = @(Apply-Managed-Kimi-Context $launchArgs)
Apply-Managed-Agy-Context $launchArgs
Apply-Managed-Kiro-Context $launchArgs

if ($Tool -eq "omp") {
  $launchArgs += @("--config", (Join-Path $wrapperDir "managed-config\omp.yml"))
}

$launchModel = Extract-Model $launchArgs
if ($Tool -eq 'kiro-cli' -and [string]::IsNullOrWhiteSpace($launchModel)) { $launchModel=$configuredModel }
$resumeTarget = Extract-Resume-Target $launchArgs
$transcriptPath = if ($Tool -eq "omp") { Extract-Omp-Session-Directory $launchArgs } else { "" }
if (($Tool -eq 'kimi' -or $Tool -eq 'kimi-code') -and -not [string]::IsNullOrWhiteSpace($env:CODUX_NATIVE_KIMI_CONTEXT_MANIFEST)) {
  $kimiHome = if (-not [string]::IsNullOrEmpty($env:KIMI_CODE_HOME)) { $env:KIMI_CODE_HOME } else { Join-Path $env:USERPROFILE '.kimi-code' }
  $kimiHome = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($kimiHome)
  $transcriptPath = Join-Path $kimiHome 'session_index.jsonl'
}
$bindingExternalSessionId = if (-not [string]::IsNullOrWhiteSpace($resumeTarget)) { $resumeTarget } else { $env:DMUX_EXTERNAL_SESSION_ID }
$bindingSessionOrigin = if (-not [string]::IsNullOrWhiteSpace($resumeTarget)) { "restored" } else { "" }
$env:DMUX_ACTIVE_AI_MODEL = $launchModel
if (-not [string]::IsNullOrWhiteSpace($resumeTarget)) {
  $env:DMUX_EXTERNAL_SESSION_ID = $resumeTarget
}

if ($Tool -eq "opencode" -or $Tool -eq "mimo") {
  $openCodeConfigDir = Join-Path $wrapperDir "opencode-config"
  if ($Tool -eq "mimo") {
    $env:XDG_CONFIG_HOME = Join-Path $openCodeConfigDir "xdg"
  } else {
    $env:OPENCODE_CONFIG_DIR = $openCodeConfigDir
  }
  $env:DMUX_ACTIVE_AI_TOOL = $Tool
}

if ($Tool -eq "kimi" -or $Tool -eq "kimi-code") {
  $env:TERM_PROGRAM = "ghostty"
  $env:DMUX_ACTIVE_AI_TOOL = $Tool
}

Apply-Managed-Lifecycle-Env $Tool
Apply-Managed-CodeWhale-Context $launchArgs

$launchDir = ""
Write-Runtime-Binding $bindingExternalSessionId $launchModel $transcriptPath $bindingSessionOrigin
Invoke-Real-Binary $realBin $launchArgs $runtimePath $launchDir
$exitCode = if ($null -eq $script:DMUX_WRAPPER_EXIT_CODE) { 0 } else { $script:DMUX_WRAPPER_EXIT_CODE }
Emit-Wrapper-SessionEnd
exit $exitCode
