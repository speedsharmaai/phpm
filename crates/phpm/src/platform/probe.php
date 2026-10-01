<?php
// What Composer's PlatformRepository sees, as JSON on stdout. The library
// detection is ported from Composer 2.10.3 src/Composer/Repository/PlatformRepository.php
// and src/Composer/Platform/Version.php (MIT, (c) Nils Adermann, Jordi Boggiano).

function phpm_info($name) {
    $reflector = new ReflectionExtension($name);
    ob_start();
    $reflector->info();
    return (string) ob_get_clean();
}

function phpm_alpha($alpha) {
    return strlen($alpha) * (-ord('a') + 1) + array_sum(array_map('ord', str_split($alpha)));
}

function phpm_openssl($version, &$isFips) {
    $isFips = false;
    if (!preg_match('/^(?<version>[0-9.]+)(?<patch>[a-z]{0,2})(?<suffix>(?:-?(?:dev|pre|alpha|beta|rc|fips)[\d]*)*)(?:-\w+)?(?: \(.+?\))?$/', $version, $m)) {
        return null;
    }
    $patch = '';
    if (version_compare($m['version'], '3.0.0', '<')) {
        $patch = '.'.phpm_alpha($m['patch']);
    }
    $isFips = strpos($m['suffix'], 'fips') !== false;
    $suffix = strtr('-'.ltrim($m['suffix'], '-'), array('-fips' => '', '-pre' => '-alpha'));
    return rtrim($m['version'].$patch.$suffix, '-');
}

function phpm_version_id($id) {
    return sprintf('%d.%d.%d', $id / 10000, (int) ($id / 100) % 100, $id % 100);
}

$libs = array();
function phpm_lib($name, $version, $replaces = array(), $provides = array()) {
    global $libs;
    if ($version !== null && $version !== false) {
        $libs[] = array($name, (string) $version, $replaces, $provides);
    }
}

$loaded = get_loaded_extensions();
$extensions = array();
foreach ($loaded as $name) {
    if (in_array($name, array('standard', 'Core'))) {
        continue;
    }
    $v = phpversion($name);
    $extensions[] = array($name, $v === false ? '0' : $v);
}

foreach ($loaded as $name) {
    switch ($name) {
        case 'amqp':
            $info = phpm_info($name);
            if (preg_match('/^librabbitmq version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-librabbitmq', $m['version']);
            }
            if (preg_match('/^AMQP protocol version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-protocol', str_replace('-', '.', $m['version']));
            }
            break;
        case 'bz2':
            if (preg_match('/^BZip2 Version => (?<version>.*),/im', phpm_info($name), $m)) {
                phpm_lib($name, $m['version']);
            }
            break;
        case 'curl':
            $curl = curl_version();
            phpm_lib($name, $curl['version']);
            $info = phpm_info($name);
            if (preg_match('{^SSL Version => (?<library>[^\r\n/]+)/(?<version>[^\r\n]+?)\r?$}im', $info, $m)) {
                $library = strtolower($m['library']);
                if ($library === 'openssl') {
                    $parsed = phpm_openssl($m['version'], $isFips);
                    phpm_lib($name.'-openssl'.($isFips ? '-fips' : ''), $parsed, array(), $isFips ? array('curl-openssl') : array());
                } else {
                    if (strpos($library, '(securetransport)') === 0 && preg_match('{^\(securetransport\) ([a-z0-9]+)}', $library, $st)) {
                        $short = 'securetransport';
                        $sslLib = 'curl-'.$st[1];
                    } else {
                        $short = $library;
                        $sslLib = 'curl-openssl';
                    }
                    phpm_lib($name.'-'.$short, $m['version'], array($sslLib));
                }
            }
            if (preg_match('{^libSSH Version => (?<library>[^\r\n/]+)/(?<version>.+?)(?:/.*)?$}im', $info, $m)) {
                phpm_lib($name.'-'.strtolower($m['library']), $m['version']);
            }
            if (preg_match('{^ZLib Version => (?<version>.+)$}im', $info, $m)) {
                phpm_lib($name.'-zlib', $m['version']);
            }
            break;
        case 'date':
            $info = phpm_info($name);
            if (preg_match('/^timelib version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-timelib', $m['version']);
            }
            if (preg_match('/^Timezone Database => (?<source>internal|external)$/im', $info, $src)) {
                $external = $src['source'] === 'external';
                if (preg_match('/^"Olson" Timezone Database Version => (?<version>.+?)(?:\.system)?$/im', $info, $m)) {
                    if ($external && in_array('timezonedb', $loaded, true)) {
                        phpm_lib('timezonedb-zoneinfo', $m['version'], array($name.'-zoneinfo'));
                    } else {
                        phpm_lib($name.'-zoneinfo', $m['version']);
                    }
                }
            }
            break;
        case 'fileinfo':
            if (preg_match('/^libmagic => (?<version>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-libmagic', $m['version']);
            }
            break;
        case 'gd':
            phpm_lib($name, constant('GD_VERSION'));
            $info = phpm_info($name);
            if (preg_match('/^libJPEG Version => (?<version>.+?)(?: compatible)?$/im', $info, $m)) {
                phpm_lib($name.'-libjpeg', preg_match('/^(?<major>\d+)(?<minor>[a-z]*)$/', $m['version'], $j) ? $j['major'].'.'.phpm_alpha($j['minor']) : null);
            }
            if (preg_match('/^libPNG Version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-libpng', $m['version']);
            }
            if (preg_match('/^FreeType Version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-freetype', $m['version']);
            }
            if (preg_match('/^libXpm Version => (?<versionId>\d+)$/im', $info, $m)) {
                phpm_lib($name.'-libxpm', phpm_version_id((int) $m['versionId']));
            }
            break;
        case 'gmp':
            phpm_lib($name, constant('GMP_VERSION'));
            break;
        case 'iconv':
            phpm_lib($name, constant('ICONV_VERSION'));
            break;
        case 'intl':
            $info = phpm_info($name);
            if (defined('INTL_ICU_VERSION')) {
                phpm_lib('icu', constant('INTL_ICU_VERSION'));
            } elseif (preg_match('/^ICU version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib('icu', $m['version']);
            }
            if (preg_match('/^ICU TZData version => (?<version>.*)$/im', $info, $m) && preg_match('/^(?<year>\d{4})(?<revision>[a-z]*)$/', $m['version'], $z)) {
                phpm_lib('icu-zoneinfo', $z['year'].'.'.phpm_alpha($z['revision']));
            }
            if (class_exists('ResourceBundle', false)) {
                $bundle = ResourceBundle::create('root', 'ICUDATA', false);
                if ($bundle !== null) {
                    phpm_lib('icu-cldr', $bundle->get('Version'));
                }
            }
            if (class_exists('IntlChar', false)) {
                phpm_lib('icu-unicode', implode('.', array_slice(IntlChar::getUnicodeVersion(), 0, 3)));
            }
            break;
        case 'imagick':
            $im = new Imagick();
            $v = $im->getVersion();
            if (preg_match('/^ImageMagick (?<version>[\d.]+)(?:-(?<patch>\d+))?/', $v['versionString'], $m)) {
                phpm_lib($name.'-imagemagick', $m['version'].(isset($m['patch']) ? '.'.$m['patch'] : ''), array('imagick'));
            }
            break;
        case 'ldap':
            $info = phpm_info($name);
            if (preg_match('/^Vendor Version => (?<versionId>\d+)$/im', $info, $m) && preg_match('/^Vendor Name => (?<vendor>.+)$/im', $info, $vm)) {
                phpm_lib($name.'-'.strtolower($vm['vendor']), phpm_version_id((int) $m['versionId']));
            }
            break;
        case 'libxml':
            $provides = array();
            foreach (array_intersect($loaded, array('dom', 'simplexml', 'xml', 'xmlreader', 'xmlwriter')) as $ext) {
                $provides[] = $ext.'-libxml';
            }
            phpm_lib($name, constant('LIBXML_DOTTED_VERSION'), array(), $provides);
            break;
        case 'mbstring':
            $info = phpm_info($name);
            if (preg_match('/^libmbfl version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-libmbfl', $m['version']);
            }
            if (PHP_VERSION_ID < 90000 && defined('MB_ONIGURUMA_VERSION')) {
                phpm_lib($name.'-oniguruma', @constant('MB_ONIGURUMA_VERSION'));
            } elseif (preg_match('/^(?:oniguruma|Multibyte regex \(oniguruma\)) version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-oniguruma', $m['version']);
            }
            break;
        case 'memcached':
            if (preg_match('/^libmemcached version => (?<version>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-libmemcached', $m['version']);
            }
            break;
        case 'openssl':
            if (preg_match('{^(?:OpenSSL|LibreSSL)?\s*(?<version>\S+)}i', constant('OPENSSL_VERSION_TEXT'), $m)) {
                $parsed = phpm_openssl($m['version'], $isFips);
                phpm_lib($name.($isFips ? '-fips' : ''), $parsed, array(), $isFips ? array($name) : array());
            }
            break;
        case 'pcre':
            phpm_lib($name, preg_replace('{^(\S+).*}', '$1', constant('PCRE_VERSION')));
            if (preg_match('/^PCRE Unicode Version => (?<version>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-unicode', $m['version']);
            }
            break;
        case 'mysqlnd':
        case 'pdo_mysql':
            if (preg_match('/^(?:Client API version|Version) => mysqlnd (?<version>.+?) /mi', phpm_info($name), $m)) {
                phpm_lib($name.'-mysqlnd', $m['version']);
            }
            break;
        case 'mongodb':
            $info = phpm_info($name);
            if (preg_match('/^libmongoc bundled version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-libmongoc', $m['version']);
            }
            if (preg_match('/^libbson bundled version => (?<version>.+)$/im', $info, $m)) {
                phpm_lib($name.'-libbson', $m['version']);
            }
            break;
        case 'pgsql':
            if (defined('PGSQL_LIBPQ_VERSION')) {
                phpm_lib('pgsql-libpq', constant('PGSQL_LIBPQ_VERSION'));
                break;
            }
            // fall through, as Composer does
        case 'pdo_pgsql':
            if (preg_match('/^PostgreSQL\(libpq\) Version => (?<version>.*)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-libpq', $m['version']);
            }
            break;
        case 'pq':
            if (preg_match('/^libpq => (?<compiled>.+) => (?<linked>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-libpq', $m['linked']);
            }
            break;
        case 'rdkafka':
            if (defined('RD_KAFKA_VERSION')) {
                $v = constant('RD_KAFKA_VERSION');
                phpm_lib($name.'-librdkafka', sprintf('%d.%d.%d', ($v & 0x7F000000) >> 24, ($v & 0x00FF0000) >> 16, ($v & 0x0000FF00) >> 8));
            }
            break;
        case 'libsodium':
        case 'sodium':
            if (defined('SODIUM_LIBRARY_VERSION')) {
                phpm_lib('libsodium', constant('SODIUM_LIBRARY_VERSION'));
            }
            break;
        case 'sqlite3':
        case 'pdo_sqlite':
            if (preg_match('/^SQLite Library => (?<version>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-sqlite', $m['version']);
            }
            break;
        case 'ssh2':
            if (preg_match('/^libssh2 version => (?<version>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name.'-libssh2', $m['version']);
            }
            break;
        case 'xsl':
            phpm_lib('libxslt', constant('LIBXSLT_DOTTED_VERSION'), array('xsl'));
            if (preg_match('/^libxslt compiled against libxml Version => (?<version>.+)$/im', phpm_info('xsl'), $m)) {
                phpm_lib('libxslt-libxml', $m['version']);
            }
            break;
        case 'yaml':
            if (preg_match('/^LibYAML Version => (?<version>.+)$/im', phpm_info('yaml'), $m)) {
                phpm_lib($name.'-libyaml', $m['version']);
            }
            break;
        case 'zip':
            if (defined('ZipArchive::LIBZIP_VERSION')) {
                phpm_lib($name.'-libzip', constant('ZipArchive::LIBZIP_VERSION'), array('zip'));
            }
            break;
        case 'zlib':
            if (defined('ZLIB_VERSION')) {
                phpm_lib($name, constant('ZLIB_VERSION'));
            } elseif (preg_match('/^Linked Version => (?<version>.+)$/im', phpm_info($name), $m)) {
                phpm_lib($name, $m['version']);
            }
            break;
    }
}

$ini = array();
$loadedIni = php_ini_loaded_file();
$ini[] = $loadedIni === false ? '' : $loadedIni;
$scanned = php_ini_scanned_files();
if ($scanned !== false) {
    foreach (explode(',', $scanned) as $file) {
        $file = trim($file);
        if ($file !== '') {
            $ini[] = $file;
        }
    }
}

echo json_encode(array(
    'version' => PHP_VERSION,
    'debug' => (bool) PHP_DEBUG,
    'zts' => defined('PHP_ZTS') && PHP_ZTS,
    'int_size' => PHP_INT_SIZE,
    'ipv6' => defined('AF_INET6') || @inet_pton('::') !== false,
    'extensions' => $extensions,
    'libraries' => $libs,
    'ini' => $ini,
), JSON_UNESCAPED_SLASHES | JSON_PARTIAL_OUTPUT_ON_ERROR);
