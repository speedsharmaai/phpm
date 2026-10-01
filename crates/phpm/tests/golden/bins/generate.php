<?php
// Regenerates these files with Composer's own BinaryInstaller: php generate.php, then copy p/vendor/bin/* here ("it's" is stored as "its").
Phar::loadPhar('/opt/homebrew/bin/composer', 'composer.phar');
require 'phar://composer.phar/vendor/autoload.php';
umask(022);
$root = __DIR__ . '/p';
@mkdir($root, 0777, true);
$root = realpath($root);
$vendor = $root . '/vendor';
$bins = [
  'a/plain' => ['bin/plain' => "<?php\necho 1;\n"],
  'a/spaced' => ['bin/spaced' => "\n\t <?php\n"],
  'a/shebang' => ['bin/tool' => "#!/usr/bin/env php\n<?php\necho 1;\n"],
  'a/crlf' => ['bin/crlf' => "#!/usr/bin/php -d x=1\r\n<?php\n"],
  'phpunit/phpunit' => ['phpunit' => "#!/usr/bin/env php\n<?php\n"],
  'a/sh' => ['bin/run.sh' => "#!/bin/bash\necho hi\n", "bin/it's" => "echo quoted\n"],
];
foreach ($bins as $name => $files) {
  $path = $vendor . '/' . $name;
  foreach ($files as $rel => $content) {
    @mkdir(dirname($path . '/' . $rel), 0777, true);
    file_put_contents($path . '/' . $rel, $content);
  }
  $package = new Composer\Package\Package($name, '1.0.0.0', '1.0.0');
  $package->setBinaries(array_keys($files));
  $installer = new Composer\Installer\BinaryInstaller(new Composer\IO\NullIO(), $vendor . '/bin', 'full', null, $vendor);
  $installer->installBinaries($package, $path);
}
