<?php

declare(strict_types=1);

namespace Speedsharma\Phpm;

final class UnsupportedPlatformException extends \RuntimeException
{
}

/**
 * Downloads the phpm release binary matching the current platform into
 * bin/.bin_real/<version>/, the same shape as the npm wrapper's
 * node_modules/.bin_real (dist/npm-wrapper/binary-install.js).
 */
final class Installer
{
    private const TARGETS = [
        'Darwin:arm64' => 'aarch64-apple-darwin',
        'Darwin:x86_64' => 'x86_64-apple-darwin',
        'Linux:aarch64' => 'aarch64-unknown-linux-musl',
        'Linux:arm64' => 'aarch64-unknown-linux-musl',
        'Linux:x86_64' => 'x86_64-unknown-linux-musl',
        'Windows:AMD64' => 'x86_64-pc-windows-msvc',
    ];

    public static function platformTarget(?string $osFamily = null, ?string $machine = null): string
    {
        $osFamily ??= PHP_OS_FAMILY;
        $machine ??= php_uname('m');
        $key = "$osFamily:$machine";

        if (!isset(self::TARGETS[$key])) {
            throw new UnsupportedPlatformException(sprintf(
                'phpm has no release binary for %s/%s. Supported targets: %s',
                $osFamily,
                $machine,
                implode(', ', array_unique(array_values(self::TARGETS))),
            ));
        }

        return self::TARGETS[$key];
    }

    public static function artifactName(string $target): string
    {
        $ext = str_contains($target, 'windows') ? 'zip' : 'tar.xz';

        return "phpm-$target.$ext";
    }

    public static function binaryName(string $target): string
    {
        return str_contains($target, 'windows') ? 'phpm.exe' : 'phpm';
    }

    public static function cacheDir(string $packageRoot, string $version): string
    {
        return "$packageRoot/bin/.bin_real/$version";
    }

    public static function binaryPath(string $packageRoot, string $version, ?string $target = null): string
    {
        $target ??= self::platformTarget();

        return self::cacheDir($packageRoot, $version) . '/' . self::binaryName($target);
    }

    public static function downloadUrl(string $version, string $target): string
    {
        return sprintf(
            'https://github.com/speedsharmaai/phpm/releases/download/v%s/%s',
            $version,
            self::artifactName($target),
        );
    }

    public static function packageVersion(string $composerJsonPath): string
    {
        $raw = json_decode((string) file_get_contents($composerJsonPath), true);

        return $raw['version'] ?? getenv('PHPM_VERSION') ?: '0.0.0';
    }

    /**
     * Composer `post-install-cmd` / `post-update-cmd` entry point. Best
     * effort: a failure here (offline, --no-scripts, an unreleased
     * platform) is not fatal, because bin/phpm downloads lazily on first
     * run too.
     *
     * @param \Composer\Script\Event $event
     */
    public static function install($event): void
    {
        $io = $event->getIO();

        try {
            $root = dirname(__DIR__);
            $version = self::packageVersion("$root/composer.json");
            $path = self::ensureInstalled($root, $version);
            $io->write("<info>phpm $version ready at $path</info>");
        } catch (\Throwable $e) {
            $io->writeError('<warning>phpm: ' . $e->getMessage() . '</warning>');
        }
    }

    public static function ensureInstalled(string $packageRoot, string $version, ?string $target = null): string
    {
        $target ??= self::platformTarget();
        $path = self::binaryPath($packageRoot, $version, $target);

        if (is_file($path) && is_executable($path)) {
            return $path;
        }

        self::download(self::downloadUrl($version, $target), self::artifactName($target), dirname($path));

        if (!is_file($path)) {
            throw new \RuntimeException("phpm binary missing after extraction: $path");
        }

        chmod($path, 0o755);

        return $path;
    }

    private static function download(string $url, string $artifactName, string $destDir): void
    {
        if (!is_dir($destDir) && !mkdir($destDir, 0o755, true) && !is_dir($destDir)) {
            throw new \RuntimeException("could not create $destDir");
        }

        $tmp = tempnam(sys_get_temp_dir(), 'phpm-');
        $context = stream_context_create(['http' => ['follow_location' => 1, 'timeout' => 30]]);
        $data = file_get_contents($url, false, $context);

        if ($data === false) {
            throw new \RuntimeException("failed to download $url");
        }

        file_put_contents($tmp, $data);
        self::extract($tmp, $artifactName, $destDir);
        unlink($tmp);
    }

    private static function extract(string $archivePath, string $artifactName, string $destDir): void
    {
        if (str_ends_with($artifactName, '.zip')) {
            $zip = new \ZipArchive();

            if ($zip->open($archivePath) !== true) {
                throw new \RuntimeException("failed to open $artifactName");
            }

            $zip->extractTo($destDir);
            $zip->close();

            return;
        }

        $command = 'tar xf ' . escapeshellarg($archivePath) . ' --strip-components 1 -C ' . escapeshellarg($destDir);
        exec($command, $output, $status);

        if ($status !== 0) {
            throw new \RuntimeException("failed to extract $artifactName: " . implode("\n", $output));
        }
    }

    public static function run(string $binaryPath, array $args): int
    {
        $command = escapeshellarg($binaryPath);

        foreach ($args as $arg) {
            $command .= ' ' . escapeshellarg($arg);
        }

        passthru($command, $status);

        return $status;
    }
}
