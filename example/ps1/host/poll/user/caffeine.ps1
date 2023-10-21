<#
  .SYNOPSIS
    caffeine
  .DESCRIPTION
    スリーブ抑止する
  .INPUTS
    - $mode: 動作モード
             "register": タスク登録 (デフォルト)
             "main": メイン処理
  .OUTPUTS
    - 0: SUCCESS / 1: ERROR
  .Last Change : 2023/10/20 20:10:09.
#>
param([string]$mode = "register", [bool]$async = $false)
$ErrorActionPreference = "Stop"
$DebugPreference = "SilentlyContinue" # Continue SilentlyContinue Stop Inquire
$version = "20231020_201009"
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
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <GroupId>S-1-5-32-545</GroupId>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <Duration>PT10M</Duration>
      <WaitTimeout>PT1H</WaitTimeout>
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
    <ExecutionTimeLimit>PT72H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>C:\Windows\system32\wscript.exe</Command>
      <Arguments>"$($app.spyrunDir)\launch.js" "$($app.cmdLocalFile)" "$($app.cmdLocalDir)"</Arguments>
      <WorkingDirectory>$($app.spyrunDir)</WorkingDirectory>
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
    if (!$async) {
      $launch = [System.IO.Path]::Combine($app.spyrunDir, "launch.js")
      Start-Process -File "wscript.exe" -ArgumentList $launch, $app.cmdFile, $mode, 1
      $app.result = $app.cnst.SUCCESS
      return
    }
    log "[info] Currently ordering a double shot of espresso..."

    $sig = @"
[DllImport("kernel32.dll", CharSet = CharSet.Auto, SetLastError = true)]
public static extern void SetThreadExecutionState(uint esFlags);
"@

    $ES_CONTINUOUS = [uint32]"0x80000000"
    $ES_AWAYMODE_REQUIRED = [uint32]"0x00000040"
    $ES_DISPLAY_REQUIRED = [uint32]"0x00000002"
    $ES_SYSTEM_REQUIRED = [uint32]"0x00000001"

    $jName = "DrinkALotOfEspresso"

    $stes = Add-Type -MemberDefinition $sig -Name System -Namespace Win32 -PassThru

    [void]$stes::SetThreadExecutionState($ES_SYSTEM_REQUIRED -bor $ES_DISPLAY_REQUIRED -bor $ES_CONTINUOUS)

    Read-Host "[info] Enter if you want to exit ..."
    log "[info] No more espressos left behind the counter."

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

