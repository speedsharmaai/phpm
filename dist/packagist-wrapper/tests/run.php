<?php

declare(strict_types=1);

// Standalone smoke test, no PHPUnit dependency: run with `php tests/run.php`.
// Covers Installer's pure platform/path logic; download() and extract() are
// not exercised here (network, and tar/zip binaries), the same split the
// npm wrapper's test.sh makes.

require __DIR__ . '/../src/Installer.php';

use Speedsharma\Phpm\Installer;
use Speedsharma\Phpm\UnsupportedPlatformException;

$failures = 0;

function check(string $name, $want, $got): void
{
    global $failures;

    if ($want === $got) {
        echo "ok   $name\n";

        return;
    }

    $failures++;
    echo "FAIL $name\n";
    echo '  want: ' . var_export($want, true) . "\n";
    echo '  got:  ' . var_export($got, true) . "\n";
}

check('maps apple silicon to the macOS aarch64 target', 'aarch64-apple-darwin', Installer::platformTarget('Darwin', 'arm64'));
check('maps intel macs to the macOS x86_64 target', 'x86_64-apple-darwin', Installer::platformTarget('Darwin', 'x86_64'));
check('maps linux arm64 to the musl aarch64 target', 'aarch64-unknown-linux-musl', Installer::platformTarget('Linux', 'arm64'));
check('maps linux x86_64 to the musl x86_64 target', 'x86_64-unknown-linux-musl', Installer::platformTarget('Linux', 'x86_64'));
check('maps windows amd64 to the msvc target', 'x86_64-pc-windows-msvc', Installer::platformTarget('Windows', 'AMD64'));

$threw = null;
try {
    Installer::platformTarget('Linux', 'riscv64');
} catch (UnsupportedPlatformException $e) {
    $threw = $e->getMessage();
}
check('an unsupported platform names what it does support', true, $threw !== null && str_contains($threw, 'aarch64-apple-darwin'));

check('a tar.xz artifact name for a unix target', 'phpm-x86_64-apple-darwin.tar.xz', Installer::artifactName('x86_64-apple-darwin'));
check('a zip artifact name for the windows target', 'phpm-x86_64-pc-windows-msvc.zip', Installer::artifactName('x86_64-pc-windows-msvc'));

check('the unix binary has no extension', 'phpm', Installer::binaryName('aarch64-unknown-linux-musl'));
check('the windows binary is phpm.exe', 'phpm.exe', Installer::binaryName('x86_64-pc-windows-msvc'));

check('the cache dir is versioned under the package root', '/pkg/bin/.bin_real/0.1.0', Installer::cacheDir('/pkg', '0.1.0'));
check('the binary path adds the per-target binary name', '/pkg/bin/.bin_real/0.1.0/phpm.exe', Installer::binaryPath('/pkg', '0.1.0', 'x86_64-pc-windows-msvc'));

check(
    'the download url points at the tagged GitHub release asset',
    'https://github.com/speedsharmaai/phpm/releases/download/v0.1.0/phpm-aarch64-apple-darwin.tar.xz',
    Installer::downloadUrl('0.1.0', 'aarch64-apple-darwin'),
);

$tmpComposerJson = tempnam(sys_get_temp_dir(), 'phpm-composer-');
file_put_contents($tmpComposerJson, json_encode(['version' => '9.9.9']));
check('package version reads composer.json\'s version field', '9.9.9', Installer::packageVersion($tmpComposerJson));
unlink($tmpComposerJson);

if ($failures > 0) {
    echo "$failures failed\n";
    exit(1);
}
