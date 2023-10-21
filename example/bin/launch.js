try {
  var shell = WScript.CreateObject("WScript.Shell");
  var args = WScript.Arguments;

  var cmds = [];
  for (var i = 0; i < args.length; i++) {
    cmds.push(args(i));
  }

  var code = shell.Run(cmds.join(" "), 0, true);
  WScript.Quit(code);
} catch (e) {
  WScript.Quit(1);
}
