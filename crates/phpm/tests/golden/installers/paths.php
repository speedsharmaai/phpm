<?php
// Usage: php golden_installers.php <project-with-composer/installers-2.3.0-and-composer/composer> < cases.json > golden.json
require $argv[1] . '/vendor/autoload.php';

use Composer\Composer;
use Composer\Config;
use Composer\Installers\Installer;
use Composer\IO\NullIO;
use Composer\Package\Package;
use Composer\Package\RootPackage;

$cases = json_decode(stream_get_contents(STDIN), true);
$cwd = getcwd();
$out = [];
foreach ($cases as $case) {
    $composer = new Composer();
    $config = new Config(false, $cwd);
    $composer->setConfig($config);
    $root = new RootPackage('root/root', '1.0.0.0', '1.0.0');
    $root->setExtra($case['root_extra'] ?? []);
    $composer->setPackage($root);
    $composer->setDownloadManager(new \Composer\Downloader\DownloadManager(new NullIO()));
    $result = null;
    try {
        $installer = new Installer(new NullIO(), $composer);
        $version = $case['version'] ?? '1.0.0';
        $parser = new \Composer\Semver\VersionParser();
        $package = new Package($case['name'], $parser->normalize($version), $version);
        $package->setType($case['type']);
        $package->setExtra($case['extra'] ?? []);
        if ($installer->supports($case['type'])) {
            $path = $installer->getInstallPath($package);
            $result = str_starts_with($path, $cwd . '/') ? '{cwd}/' . substr($path, strlen($cwd) + 1) : $path;
        }
    } catch (\Throwable $e) {
        $result = ['error' => get_class($e)];
    }
    $case['path'] = $result;
    $out[] = $case;
}
echo json_encode($out, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES), "\n";
