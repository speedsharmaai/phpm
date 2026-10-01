<?php
// Regenerate paths.json, with a project that has composer/installers 2.3.0
// and composer/composer installed (the drupal-recommended fixture's dev set):
//   php cases.php <project> | php paths.php <project> > paths.json
require $argv[1] . '/vendor/autoload.php';
$r = new ReflectionClass(Composer\Installers\Installer::class);
$types = $r->getDefaultProperties()['supportedTypes'];
$names = ['acme/my-cool_Plugin', 'Some.Vendor/oc-wn-ti-module-x-plugin-module', 'grav/grav-theme-Fancy-thing-template'];
$cases = [];
foreach ($types as $fw => $cls) {
    if (in_array($fw, ['cakephp', 'bitrix'])) continue;
    $c = new ReflectionClass('Composer\\Installers\\'.$cls);
    $p = $c->getDefaultProperties();
    $locs = $p['locations'];
    if ($fw === 'ee2') $locs = $p['ee2Locations'];
    if ($fw === 'ee3') $locs = $p['ee3Locations'];
    foreach (array_keys($locs) as $loc) {
        if ($fw === 'oxid' && $loc === 'module') continue;
        foreach ($names as $n) $cases[] = ['name' => $n, 'type' => "$fw-$loc"];
    }
}
$cases[] = ['name' => 'a/x', 'type' => 'drupal-module', 'root_extra' => ['installer-paths' => ['web/m/{$vendor}/{$name}/' => ['type:drupal-module']]]];
$cases[] = ['name' => 'A/X', 'type' => 'drupal-module', 'root_extra' => ['installer-paths' => ['by-name/{$name}' => 'A/X', 'by-type/{$name}' => ['type:drupal-module']]]];
$cases[] = ['name' => 'v/x', 'type' => 'wordpress-plugin', 'root_extra' => ['installer-paths' => ['/abs/{$type}/{$name}' => ['vendor:v']]]];
$cases[] = ['name' => 'a/my-plugin', 'type' => 'matomo-plugin', 'extra' => ['installer-name' => 'Named']];
$cases[] = ['name' => 'a/x', 'type' => 'drupal-module', 'root_extra' => ['installer-disable' => ['drupal']]];
$cases[] = ['name' => 'a/x', 'type' => 'drupal-module', 'root_extra' => ['installer-disable' => true]];
$cases[] = ['name' => 'a/x', 'type' => 'tao-extension', 'extra' => ['tao-extension-name' => 'taoX']];
$cases[] = ['name' => 'a/x', 'type' => 'mautic-plugin', 'extra' => ['install-directory-name' => 'Dir']];
$cases[] = ['name' => 'a/x', 'type' => 'tastyigniter-extension', 'extra' => ['tastyigniter-extension' => ['code' => 'igniter.cart']]];
$cases[] = ['name' => 'a/x', 'type' => 'tastyigniter-theme', 'extra' => ['tastyigniter-theme' => ['code' => 'ti-theme-orange']]];
$cases[] = ['name' => 'silverstripe/framework', 'type' => 'silverstripe-module', 'version' => '2.4.7'];
$cases[] = ['name' => 'silverstripe/framework', 'type' => 'silverstripe-module', 'version' => '3.1.0'];
$cases[] = ['name' => 'a/x', 'type' => 'wordpress-plugin-extra'];
$cases[] = ['name' => 'a/x', 'type' => 'fork-cms-module'];
$cases[] = ['name' => 'a/x', 'type' => 'library'];
echo json_encode($cases, JSON_UNESCAPED_SLASHES);
