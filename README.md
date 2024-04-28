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

## [vars]

Variables can be set freely.
Variables are defined in alphabetical order.
Therefore, it is necessary to pay attention to the order.

- OK
```toml
[vars]
a = "a"
b = "b and {{ a }}"
```

- NG
```toml
[vars]
a = "a and {{ b }}"
b = "b"
```

## [cfg]

### stop_flg

The file path to stop the spyrun.
When it detects that this path has been created or modified,
it completes all running operations and exits.

### stop_force_flg

The file path to force stop the spyrun.
When it detects that this path has been created or modified,
it forces the spyrun to stop immediately.

### max_threads

The maximum number of threads to use in the spyrun.
The default value is based on [the number of CPU cores](https://github.com/rayon-rs/rayon/blob/main/FAQ.md#how-many-threads-will-rayon-spawn).

## [log]

### path

The path to the log file.

### level

The log level.
The default value is `info`.
You can specify the following values.

- off
- error
- warn
- info
- debug
- trace

## [init]

Init is executed when spyrun starts.

### cmd

The command to execute.

### arg

The arguments to pass to the command.

## [[spys]]

The list of spy.

### name

The name of the spy.

### events

The list of events.
Default value is ['Create', 'Modify'].
You can specify the following values.

- Access
- Create
- Modify
- Remove

### input

The path to watch.

### output

The path to output.
Standard output and standard error is written to this path.

### recursive

If you want to watch the input path recursively, set this to true.
Default value is false.

### debounce

If you want to debounce execution, set this setting.
Default value is 50 milliseconds.

### throttle

If you want to throttle execution, set this setting.
Default value is 0 milliseconds.

### delay

The delay to wait before executing the command.
Default value is 0 milliseconds.

- one param

Execute after 5000 milliseconds.

```toml
delay = [5000]
```

- two params

Waits randomly between 5000 milliseconds and 10000 milliseconds before executing.

```toml
delay = [5000, 10000]
```

### [[spys.patterns]]

The list of patterns.

#### pattern

The pattern to watch.
This is a regular expression.

#### cmd

The command to execute.

#### arg

The arguments to pass to the command.

### [spys.poll]

If you want to watch the input path in a polling mode, set this setting.

#### interval

The interval to watch the input path.

### [spys.walk]

If you want to walk the input path, set this setting.
If this is set, the input path is also walked when spyrun starts.

### min_depth

The minimum depth to walk the input path.

### max_depth

The maximum depth to walk the input path.

### follow_symlinks

If you want to follow symlinks, set this setting.

### pattern

The pattern to match the input path.
This is a regular expression.

#### delay

The delay to wait before walking the input path.

- one param

Walk after 5000 milliseconds.

```toml
delay = [5000]
```

- two params

Waits randomly between 5000 milliseconds and 10000 milliseconds.

```toml
delay = [5000, 10000]
```

# License

spyrun is distributed under the MIT License.

