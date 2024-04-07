# spyrun - file watcher and command executor

spyrun is a tool that watches files and executes commands when specific events occur.
It can watch for file modifications, additions, and more.

# Installation

```
cargo install --git https://github.com/yukimemi/spyrun
```

# Usage

spyrun operates using a configuration file.

```sh
> spyrun --help
Usage: spyrun.exe [OPTIONS]

Options:
  -c, --config <FILE>  Sets a custom config file [default: spyrun.toml]
  -d, --debug...       Turn debugging information on
  -h, --help           Print help
  -V, --version        Print version
```

# Configuration File

spyrun's configuration file is in TOML format.
The default filename is `spyrun.toml`, located in the same directory as the executable.
The configuration file specifies the files to watch, the commands to execute, and various other options.

- example

```toml
[vars]
base = '{{ cwd }}/example'
hostname = '{{ env(arg="COMPUTERNAME") }}'
version = '20240407_125639'
fn_toast = '''
function Show-Toast {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory=$true)][String]$title,
    [Parameter(Mandatory=$true)][String]$message,
    [Parameter(Mandatory=$true)][String]$detail
  )
  [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
  [Windows.UI.Notifications.ToastNotification, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
  [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null

  $app_id = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
  $content = @"
<?xml version="1.0" encoding="utf-8"?>
<toast>
    <visual>
        <binding template="ToastGeneric">
            <text>$($title)</text>
            <text>$($message)</text>
            <text>$($detail)</text>
        </binding>
    </visual>
</toast>
"@
  $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
  $xml.LoadXml($content)
  $toast = New-Object Windows.UI.Notifications.ToastNotification $xml
  [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app_id).Show($toast)
}
'''

[cfg]
stop_flg = '{{ base }}/stop.flg'
stop_force_flg = '{{ base }}/stop.force.flg'
max_threads = 8

[log]
path = '{{ base }}/log/{{ cmd_stem }}.log'
level = 'info'

[init]
cmd = 'powershell'
arg = ['-NoProfile', '-Command', '''& {
  {{ fn_toast }}
  Show-Toast -title "spyrun {{ version }} on {{ hostname }}" -message "spyrun is running" -detail "spyrun path is {{ cmd_path }}. config path is {{ cfg_path }}."
}''']

# watch files and notifiy.
[[spys]]
name = 'toast'
input = '{{ base }}/watch_dir'
output = '{{ base }}/log'
[[spys.patterns]]
pattern = '\.txt$'
cmd = 'powershell'
arg = ['-NoProfile', '-Command', '''& {
  {{ fn_toast }}
  Show-Toast -title "{{ event_name }}" -message "{{ event_path }} is {{ event_kind }}" -detail "name: {{ event_name }}. dir: {{ event_dir }}"
}''']
```

# License

spyrun is distributed under the MIT License.

