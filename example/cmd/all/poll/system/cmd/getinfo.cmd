@set __SCRIPTPATH=%~f0&@powershell -NoProfile -ExecutionPolicy ByPass -InputFormat None "$s=[scriptblock]::create((gc -enc utf8 -li \"%~f0\"|?{$_.readcount -gt 2})-join\"`n\");&$s" %*
@exit /b %errorlevel%

<#
  .SYNOPSIS
    getinfo
  .DESCRIPTION
    情報取得を行う
  .INPUTS
    - $mode: 動作モード
             "register": タスク登録 (デフォルト)
             "main": メイン処理
  .OUTPUTS
    - 0: SUCCESS / 1: ERROR
  .Last Change : 2023/10/17 19:19:27.
#>
param([string]$mode = "register")
$ErrorActionPreference = "Stop"
$DebugPreference = "SilentlyContinue" # Continue SilentlyContinue Stop Inquire
$version = "20231017_191927"
# Enable-RunspaceDebug -BreakAll

<#
  .SYNOPSIS
    log
  .DESCRIPTION
    log message
  .INPUTS
    - msg
    - color
  .OUTPUTS
    - None
#>
function log {

  [CmdletBinding()]
  [OutputType([void])]
  param([string]$msg, [string]$color)
  trap {
    Write-Host "[log] Error $_"
    throw $_
  }

  $now = Get-Date -f "yyyy/MM/dd HH:mm:ss.fff"
  if ($color) {
    Write-Host -ForegroundColor $color "${now} ${msg}"
  } else {
    Write-Host "${now} ${msg}"
  }
}

<#
  .SYNOPSIS
    New-Mutex
  .DESCRIPTION
    ミューテックスを作成する。
  .INPUTS
    - name
  .OUTPUTS
    - $true: 成功
    - $false: 失敗
#>
function New-Mutex {

  [CmdletBinding()]
  [OutputType([void])]
  param([string]$name)
  trap {
    log "[New-Mutex] Error $_" "Red"
    throw $_
  }

  $mutexName = "Global¥${name}"
  log "Create mutex name: [${mutexName}]"
  $mutex = New-Object System.Threading.Mutex($false, $mutexName)

  if (!$mutex.WaitOne(0, $false)) {
    log "2重起動です！終了します。" "Yellow"
    return $false
  } else {
    $app.mutex = $mutex
    return $true
  }
}

<#
  .SYNOPSIS
    Init
  .DESCRIPTION
    Init
  .INPUTS
    - None
  .OUTPUTS
    - None
#>
function Start-Init {

  [CmdletBinding()]
  [OutputType([void])]
  param()
  trap {
    log "[Start-Init] Error $_" "Red"
    throw $_
  }

  log "[Start-Init] Start"

  $script:app = @{}

  $cmdFullPath = & {
    if ($env:__SCRIPTPATH) {
      return [System.IO.Path]::GetFullPath($env:__SCRIPTPATH)
    } else {
      return [System.IO.Path]::GetFullPath($script:MyInvocation.MyCommand.Path)
    }
  }

  $app.Add("cmdFile", $cmdFullPath)
  $app.Add("cmdDir", [System.IO.Path]::GetDirectoryName($app.cmdFile))
  $app.Add("cmdName", [System.IO.Path]::GetFileNameWithoutExtension($app.cmdFile))
  $app.Add("cmdFileName", [System.IO.Path]::GetFileName($app.cmdFile))
  $app.Add("pwd", [System.IO.Path]::GetFullPath((Get-Location).Path))
  $app.Add("now", (Get-Date -Format "yyyyMMddTHHmmssfffffff"))

  $sp = $app.cmdFile -split "\\" | Where-Object { $_ -ne $env:COMPUTERNAME -and $_ -ne "cmd" }
  $app.Add("scope", $sp[-4])
  $app.Add("watchMode", $sp[-3])
  $app.Add("userType", $sp[-2])

  $app.Add("mutex", $null)
  $app.Add("spyrunFile", "C:\ProgramData\spyrun\bin\spyrun.exe")
  $app.Add("spyrunDir", [System.IO.Path]::GetDirectoryName($app.spyrunFile))
  $app.Add("spyrunName", [System.IO.Path]::GetFileNameWithoutExtension($app.spyrunFile))
  $app.Add("spyrunFileName", [System.IO.Path]::GetFileName($app.spyrunFile))
  $app.Add("spyrunBase", [System.IO.Path]::GetDirectoryName($app.spyrunDir))
  $app.Add("registerFlg", [System.IO.Path]::Combine($app.spyrunBase, "flg", $app.scope, $app.watchMode, $app.userType, "$($app.cmdName)_${version}.flg"))
  $app.Add("cmdLocalFile", [System.IO.Path]::Combine($app.spyrunBase, "cmd", $app.scope, $app.watchMode, $app.userType, $app.cmdFileName))
  $app.Add("cmdLocalDir", [System.IO.Path]::GetDirectoryName($app.cmdLocalFile))
  $app.Add("kickFile",  [System.IO.Path]::Combine($app.spyrunBase, "kick", $app.scope, $app.watchMode, $app.userType, "$($app.cmdName).flg"))
  $app.Add("isLocal", ($app.cmdFile -eq $app.cmdLocalFile))

  # log
  $app.Add("logDir", [System.IO.Path]::Combine($app.spyrunBase, "log"))
  $app.Add("logFile", [System.IO.Path]::Combine($app.logDir, "$($app.cmdName)_$($app.now).log"))
  $app.Add("logName", [System.IO.Path]::GetFileNameWithoutExtension($app.logFile))
  $app.Add("logFileName", [System.IO.Path]::GetFileName($app.logFile))
  New-Item -Force -ItemType Directory $app.logDir
  Start-Transcript $app.logFile

  # const value.
  $app.Add("cnst", @{
      SUCCESS = 0
      ERROR   = 1
    })

  # Init result
  $app.Add("result", $app.cnst.ERROR)

  log "[Start-Init] End"
}

<#
  .SYNOPSIS
    Ensure-ScheduledTask
  .DESCRIPTION
    タスクスケジューラ未登録の場合は登録する
  .INPUTS
    - xml (タスクスケジューラ情報)
  .OUTPUTS
    - $true: 登録した場合
    - $false: 登録済の場合
#>
function Ensure-ScheduledTask {
  [CmdletBinding()]
  [OutputType([bool])]
  param([string]$xmlStr)

  trap {
    log "[Ensure-ScheduledTask] Error $_" "Red"
    throw $_
  }

  $xml = [xml]$xmlStr

  Get-ScheduledTask | Where-Object {
    $_.URI -eq $xml.Task.RegistrationInfo.URI
  } | Set-Variable exists

  if ($null -eq $exists -or !(Test-Path $app.registerFlg)) {
    New-Item -Force -ItemType Directory $app.cmdLocalDir
    Copy-Item -Force $app.cmdFile $app.cmdLocalFile
    $part = $xml.Task.RegistrationInfo.URI -split "\\"
    $taskpath = $part[0..($part.Length - 2)] -join "\"
    $taskname = $part[-1]
    log "${taskpath}/${taskname} is not registered, so Register-ScheduledTask !"
    Register-ScheduledTask -Force -TaskPath $taskpath -TaskName $taskname -Xml $xmlStr
    New-Item -Force -ItemType Directory (Split-Path -Parent $app.registerFlg)
    New-Item -Force -ItemType File $app.registerFlg
    $app.cmdFile | Set-Content -Force -Encoding utf8 $app.registerFlg
    return $true
  }

  $false
}

<#
  .SYNOPSIS
    Remove-ScheduledTask
  .DESCRIPTION
    タスクスケジューラを削除する
  .INPUTS
    - xml (タスクスケジューラ情報)
  .OUTPUTS
    - None
#>
function Remove-ScheduledTask {
  [CmdletBinding()]
  [OutputType([void])]
  param([string]$xmlStr)

  trap {
    log "[Remove-ScheduledTask] Error $_" "Red"
    throw $_
  }

  $xml = [xml]$xmlStr

  Get-ScheduledTask | Where-Object {
    $_.URI -eq $xml.Task.RegistrationInfo.URI
  } | Set-Variable exists

  if ($null -eq $exists) {
    log "$($xml.Task.RegistrationInfo.URI) is already removed !"
    return
  }

  $part = $xml.Task.RegistrationInfo.URI -split "\\"
  $taskpath = ($part[0..($part.Length - 2)] -join "\")
  $taskname = $part[-1]
  log "Unregister-ScheduledTask ${taskpath}\${taskname}"

  $removeTasks = {
    param([object]$folder)

    if (![string]::IsNullOrEmpty($taskname)) {
      $folder.GetTasks(1) | Where-Object {
        $folder.Path -eq $taskpath -and $_.Name -eq $taskname
      } | ForEach-Object {
        log "Remove taskpath: [${taskpath}], taskname: [${taskname}]"
        $folder.DeleteTask($_.Name, $null)
      }
    }
    if ([string]::IsNullOrEmpty($taskname)) {
      $folder.GetFolders(1) | ForEach-Object {
        & $removeTasks $_
      }
      $folder.GetTasks(1) | ForEach-Object {
        $folder.DeleteTask($_.Name, $null)
      }
      if ($folder.Path -eq $taskpath -and $taskpath -ne "\") {
        log "Remove taskpath folder: [$taskpath]"
        $sch = New-Object -ComObject Schedule.Service
        [void]$sch.connect()
        $rootFolder = $sch.GetFolder("\")
        [void]$rootFolder.DeleteFolder($taskpath, $null)
      }
    }
  }

  $sch = New-Object -ComObject Schedule.Service
  $sch.connect()
  $folder = $sch.GetFolder($taskpath)
  & $removeTasks $folder
}


<#
  .SYNOPSIS
    Convert-ArrayPropertyToString
  .DESCRIPTION
    オブジェクトのプロトタイプで配列型のものを , で join して返す
  .INPUTS
    - $PSObject
  .OUTPUTS
    - $PSObject
#>
function Convert-ArrayPropertyToString {
  [CmdletBinding()]
  [OutputType([PSObject])]
  param(
    [Parameter(ValueFromPipeline=$true)]
    [psobject]$InputObject
  )

  process {
    $m = $_
    $m.PSObject.Properties.Name | ForEach-Object {
      if (($null -ne $m.$_) -and ($m.$_.GetType().Name -match ".*\[\]")) {
        $m.$_ = $m.$_ -join ","
      }
    }
    $m
  }
}

<#
  .SYNOPSIS
    Get-InfoByPsCommand
  .DESCRIPTION
    PowerShell コマンドを実行して情報を採取する
  .INPUTS
    - command: 実行コマンド
    - collect: 収集先パス
  .OUTPUTS
    - None
#>
function Get-InfoByPsCommand {
  [CmdletBinding()]
  [OutputType([void])]
  param([string]$command, [string]$collect)

  trap {
    log "[Get-InfoByPsCommand] Error $_" "Red"
    throw $_
  }

  $s = Get-Date
  log "Execute [${command}] ... start"
  $today = Get-Date -f "yyyyMMdd"
  $collectLatest = [System.IO.Path]::Combine($collect, $command, "latest", "${env:COMPUTERNAME}_${command}.csv")
  $collectToday = [System.IO.Path]::Combine($collect, $command, $today, "${env:COMPUTERNAME}_${command}_${today}.csv")
  New-Item -Force -ItemType Directory (Split-Path -Parent $collectLatest) | Out-Null
  New-Item -Force -ItemType Directory (Split-Path -Parent $collectToday) | Out-Null
  Invoke-Expression $command | Select-Object * | Convert-ArrayPropertyToString | Export-Csv -NoTypeInformation -Encoding utf8 $collectLatest
  Copy-Item -Force $collectLatest $collectToday
  $e = Get-Date
  $span = $e - $s
  log ("Execute [${command}] end ! Elaps: {0} {1:00}:{2:00}:{3:00}.{4:000}" -f $span.Days, $span.Hours, $span.Minutes, $span.Seconds, $span.Milliseconds)
}

<#
  .SYNOPSIS
    Main
  .DESCRIPTION
    Execute main
  .INPUTS
    - None
  .OUTPUTS
    - Result - 0 (SUCCESS), 1 (ERROR)
#>
function Start-Main {
  [CmdletBinding()]
  [OutputType([int])]
  param()

  try {

    Start-Init
    if (!(New-Mutex "$($app.cmdName)_$($app.isLocal)")) {
      return
    }

    $xmlStr = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <URI>\spyrun\$($app.scope)\$($app.watchMode)\$($app.userType)\$($app.cmdName)</URI>
  </RegistrationInfo>
  <Triggers>
    <TimeTrigger>
      <Repetition>
        <Interval>PT15M</Interval>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
      <StartBoundary>2023-10-01T00:00:00+09:00</StartBoundary>
      <Enabled>true</Enabled>
      <RandomDelay>PT3H</RandomDelay>
    </TimeTrigger>
    <BootTrigger>
      <Enabled>true</Enabled>
    </BootTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>S-1-5-18</UserId>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>false</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <DisallowStartOnRemoteAppSession>false</DisallowStartOnRemoteAppSession>
    <UseUnifiedSchedulingEngine>true</UseUnifiedSchedulingEngine>
    <WakeToRun>true</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>$($app.cmdLocalFile)</Command>
      <WorkingDirectory>$($app.cmdLocalDir)</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"@

    if ($app.isLocal) {
      New-Item -Force -ItemType Directory (Split-Path -Parent $app.kickFile)
      New-Item -Force -ItemType File $app.kickFile
      $app.result = $app.cnst.SUCCESS
      return
    }

    if (Ensure-ScheduledTask $xmlStr) {
      $app.result = $app.cnst.SUCCESS
      return
    }

    if ($mode -ne "main") {
      log "mode: [${mode}], so exit."
      $app.result = $app.cnst.SUCCESS
      return
    }

    # Execute main.
    $collect = [System.IO.Path]::Combine((Split-Path -Parent $app.cmdDir), "collect")
    Get-InfoByPsCommand "Get-ComputerInfo" $collect
    Get-InfoByPsCommand "Get-NetIPAddress" $collect
    Get-InfoByPsCommand "Get-NetIPConfiguration" $collect
    Get-InfoByPsCommand "Get-NetRoute" $collect
    Get-InfoByPsCommand "Get-DnsClientServerAddress" $collect
    Get-InfoByPsCommand "Get-HotFix" $collect

    $app.result = $app.cnst.SUCCESS
    return

  } catch {
    log "Error ! $_" "Red"
    # Enable-RunSpaceDebug -BreakAll
    $app.result = $app.cnst.ERROR
  } finally {
    if ($null -ne $app.mutex) {
      $app.mutex.ReleaseMutex()
      $app.mutex.Close()
      $app.mutex.Dispose()
    }
    Stop-Transcript
  }
}

# Call main.
Start-Main
exit $app.result
