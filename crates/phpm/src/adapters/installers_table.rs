// Generated from composer/installers 2.3.0: Installer::$supportedTypes in
// krsort order, each class's $locations (ExpressionEngine's per prefix).
const FRAMEWORKS: [Framework; 96] = [
    Framework {
        prefix: "zikula",
        locations: &[
            ("module", "modules/{$vendor}-{$name}/"),
            ("theme", "themes/{$vendor}-{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "zend",
        locations: &[
            ("library", "library/{$name}/"),
            ("extra", "extras/library/{$name}/"),
            ("module", "module/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "yawik",
        locations: &[("module", "module/{$name}/")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "wordpress",
        locations: &[
            ("plugin", "wp-content/plugins/{$name}/"),
            ("theme", "wp-content/themes/{$name}/"),
            ("muplugin", "wp-content/mu-plugins/{$name}/"),
            ("dropin", "wp-content/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "wolfcms",
        locations: &[("plugin", "wolf/plugins/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "winter",
        locations: &[
            ("module", "modules/{$name}/"),
            ("plugin", "plugins/{$vendor}/{$name}/"),
            ("theme", "themes/{$name}/"),
        ],
        inflect: Inflect::Winter,
    },
    Framework {
        prefix: "whmcs",
        locations: &[
            ("addons", "modules/addons/{$vendor}_{$name}/"),
            ("fraud", "modules/fraud/{$vendor}_{$name}/"),
            ("gateways", "modules/gateways/{$vendor}_{$name}/"),
            ("notifications", "modules/notifications/{$vendor}_{$name}/"),
            ("registrars", "modules/registrars/{$vendor}_{$name}/"),
            ("reports", "modules/reports/{$vendor}_{$name}/"),
            ("security", "modules/security/{$vendor}_{$name}/"),
            ("servers", "modules/servers/{$vendor}_{$name}/"),
            ("social", "modules/social/{$vendor}_{$name}/"),
            ("support", "modules/support/{$vendor}_{$name}/"),
            ("templates", "templates/{$vendor}_{$name}/"),
            ("includes", "includes/{$vendor}_{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "vanilla",
        locations: &[("plugin", "plugins/{$name}/"), ("theme", "themes/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "userfrosting",
        locations: &[("sprinkle", "app/sprinkles/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "tusk",
        locations: &[
            ("task", ".tusk/tasks/{$name}/"),
            ("command", ".tusk/commands/{$name}/"),
            ("asset", "assets/tusk/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "thelia",
        locations: &[
            ("module", "local/modules/{$name}/"),
            ("frontoffice-template", "templates/frontOffice/{$name}/"),
            ("backoffice-template", "templates/backOffice/{$name}/"),
            ("email-template", "templates/email/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "tastyigniter",
        locations: &[
            ("module", "app/{$name}/"),
            ("extension", "extensions/{$vendor}/{$name}/"),
            ("theme", "themes/{$name}/"),
        ],
        inflect: Inflect::TastyIgniter,
    },
    Framework {
        prefix: "tao",
        locations: &[("extension", "{$name}")],
        inflect: Inflect::Tao,
    },
    Framework {
        prefix: "sylius",
        locations: &[("theme", "themes/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "sydes",
        locations: &[
            ("module", "app/modules/{$name}/"),
            ("theme", "themes/{$name}/"),
        ],
        inflect: Inflect::Sydes,
    },
    Framework {
        prefix: "starbug",
        locations: &[
            ("module", "modules/{$name}/"),
            ("theme", "themes/{$name}/"),
            ("custom-module", "app/modules/{$name}/"),
            ("custom-theme", "app/themes/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "smf",
        locations: &[("module", "Sources/{$name}/"), ("theme", "Themes/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "sitedirect",
        locations: &[
            ("module", "modules/{$vendor}/{$name}/"),
            ("plugin", "plugins/{$vendor}/{$name}/"),
        ],
        inflect: Inflect::SiteDirect,
    },
    Framework {
        prefix: "silverstripe",
        locations: &[("module", "{$name}/"), ("theme", "themes/{$name}/")],
        inflect: Inflect::SilverStripe,
    },
    Framework {
        prefix: "shopware",
        locations: &[
            (
                "backend-plugin",
                "engine/Shopware/Plugins/Local/Backend/{$name}/",
            ),
            ("core-plugin", "engine/Shopware/Plugins/Local/Core/{$name}/"),
            (
                "frontend-plugin",
                "engine/Shopware/Plugins/Local/Frontend/{$name}/",
            ),
            ("theme", "templates/{$name}/"),
            ("plugin", "custom/plugins/{$name}/"),
            ("frontend-theme", "themes/Frontend/{$name}/"),
        ],
        inflect: Inflect::Shopware,
    },
    Framework {
        prefix: "roundcube",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Roundcube,
    },
    Framework {
        prefix: "reindex",
        locations: &[("theme", "themes/{$name}/"), ("plugin", "plugins/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "redaxo5",
        locations: &[
            ("addon", "redaxo/src/addons/{$name}/"),
            (
                "bestyle-plugin",
                "redaxo/src/addons/be_style/plugins/{$name}/",
            ),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "redaxo",
        locations: &[
            ("addon", "redaxo/include/addons/{$name}/"),
            (
                "bestyle-plugin",
                "redaxo/include/addons/be_style/plugins/{$name}/",
            ),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "radphp",
        locations: &[("bundle", "src/{$name}/")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "quicksilver",
        locations: &[
            ("script", "web/private/scripts/quicksilver/{$name}"),
            ("module", "web/private/scripts/quicksilver/{$name}"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "pxcms",
        locations: &[
            ("module", "app/Modules/{$name}/"),
            ("theme", "themes/{$name}/"),
        ],
        inflect: Inflect::Pxcms,
    },
    Framework {
        prefix: "puppet",
        locations: &[("module", "modules/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "processwire",
        locations: &[("module", "site/modules/{$name}/")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "prestashop",
        locations: &[("module", "modules/{$name}/"), ("theme", "themes/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "ppi",
        locations: &[("module", "modules/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "porto",
        locations: &[("container", "app/Containers/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "plentymarkets",
        locations: &[("plugin", "{$name}/")],
        inflect: Inflect::Plentymarkets,
    },
    Framework {
        prefix: "piwik",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "phpbb",
        locations: &[
            ("extension", "ext/{$vendor}/{$name}/"),
            ("language", "language/{$name}/"),
            ("style", "styles/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "phifty",
        locations: &[
            ("bundle", "bundles/{$name}/"),
            ("library", "libraries/{$name}/"),
            ("framework", "frameworks/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "oxid",
        locations: &[
            ("module", "modules/{$name}/"),
            ("theme", "application/views/{$name}/"),
            ("out", "out/{$name}/"),
        ],
        inflect: Inflect::Oxid,
    },
    Framework {
        prefix: "osclass",
        locations: &[
            ("plugin", "oc-content/plugins/{$name}/"),
            ("theme", "oc-content/themes/{$name}/"),
            ("language", "oc-content/languages/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "ontowiki",
        locations: &[
            ("extension", "extensions/{$name}/"),
            ("theme", "extensions/themes/{$name}/"),
            ("translation", "extensions/translations/{$name}/"),
        ],
        inflect: Inflect::OntoWiki,
    },
    Framework {
        prefix: "october",
        locations: &[
            ("module", "modules/{$name}/"),
            ("plugin", "plugins/{$vendor}/{$name}/"),
            ("theme", "themes/{$vendor}-{$name}/"),
        ],
        inflect: Inflect::October,
    },
    Framework {
        prefix: "moodle",
        locations: &[
            ("mod", "mod/{$name}/"),
            ("admin_report", "admin/report/{$name}/"),
            ("atto", "lib/editor/atto/plugins/{$name}/"),
            ("tool", "admin/tool/{$name}/"),
            ("assignment", "mod/assignment/type/{$name}/"),
            ("assignsubmission", "mod/assign/submission/{$name}/"),
            ("assignfeedback", "mod/assign/feedback/{$name}/"),
            ("antivirus", "lib/antivirus/{$name}/"),
            ("auth", "auth/{$name}/"),
            ("availability", "availability/condition/{$name}/"),
            ("block", "blocks/{$name}/"),
            ("booktool", "mod/book/tool/{$name}/"),
            ("cachestore", "cache/stores/{$name}/"),
            ("cachelock", "cache/locks/{$name}/"),
            ("calendartype", "calendar/type/{$name}/"),
            ("communication", "communication/provider/{$name}/"),
            ("customfield", "customfield/field/{$name}/"),
            ("fileconverter", "files/converter/{$name}/"),
            ("format", "course/format/{$name}/"),
            ("coursereport", "course/report/{$name}/"),
            ("contenttype", "contentbank/contenttype/{$name}/"),
            ("customcertelement", "mod/customcert/element/{$name}/"),
            ("datafield", "mod/data/field/{$name}/"),
            ("dataformat", "dataformat/{$name}/"),
            ("datapreset", "mod/data/preset/{$name}/"),
            ("editor", "lib/editor/{$name}/"),
            ("enrol", "enrol/{$name}/"),
            ("filter", "filter/{$name}/"),
            ("forumreport", "mod/forum/report/{$name}/"),
            ("gradeexport", "grade/export/{$name}/"),
            ("gradeimport", "grade/import/{$name}/"),
            ("gradereport", "grade/report/{$name}/"),
            ("gradingform", "grade/grading/form/{$name}/"),
            ("h5plib", "h5p/h5plib/{$name}/"),
            ("local", "local/{$name}/"),
            ("logstore", "admin/tool/log/store/{$name}/"),
            ("ltisource", "mod/lti/source/{$name}/"),
            ("ltiservice", "mod/lti/service/{$name}/"),
            ("media", "media/player/{$name}/"),
            ("message", "message/output/{$name}/"),
            ("mlbackend", "lib/mlbackend/{$name}/"),
            ("mnetservice", "mnet/service/{$name}/"),
            ("paygw", "payment/gateway/{$name}/"),
            ("plagiarism", "plagiarism/{$name}/"),
            ("portfolio", "portfolio/{$name}/"),
            ("qbank", "question/bank/{$name}/"),
            ("qbehaviour", "question/behaviour/{$name}/"),
            ("qformat", "question/format/{$name}/"),
            ("qtype", "question/type/{$name}/"),
            ("quizaccess", "mod/quiz/accessrule/{$name}/"),
            ("quiz", "mod/quiz/report/{$name}/"),
            ("report", "report/{$name}/"),
            ("repository", "repository/{$name}/"),
            ("scormreport", "mod/scorm/report/{$name}/"),
            ("search", "search/engine/{$name}/"),
            ("theme", "theme/{$name}/"),
            ("tiny", "lib/editor/tiny/plugins/{$name}/"),
            ("tinymce", "lib/editor/tinymce/plugins/{$name}/"),
            ("profilefield", "user/profile/field/{$name}/"),
            ("webservice", "webservice/{$name}/"),
            ("workshopallocation", "mod/workshop/allocation/{$name}/"),
            ("workshopeval", "mod/workshop/eval/{$name}/"),
            ("workshopform", "mod/workshop/form/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "modxevo",
        locations: &[
            ("snippet", "assets/snippets/{$name}/"),
            ("plugin", "assets/plugins/{$name}/"),
            ("module", "assets/modules/{$name}/"),
            ("template", "assets/templates/{$name}/"),
            ("lib", "assets/lib/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "modx",
        locations: &[("extra", "core/packages/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "modulework",
        locations: &[("module", "modules/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "microweber",
        locations: &[
            ("module", "userfiles/modules/{$install_item_dir}/"),
            (
                "module-skin",
                "userfiles/modules/{$install_item_dir}/templates/",
            ),
            ("template", "userfiles/templates/{$install_item_dir}/"),
            ("element", "userfiles/elements/{$install_item_dir}/"),
            ("vendor", "vendor/{$install_item_dir}/"),
            ("components", "components/{$install_item_dir}/"),
        ],
        inflect: Inflect::Microweber,
    },
    Framework {
        prefix: "miaoxing",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "mediawiki",
        locations: &[
            ("core", "core/"),
            ("extension", "extensions/{$name}/"),
            ("skin", "skins/{$name}/"),
        ],
        inflect: Inflect::MediaWiki,
    },
    Framework {
        prefix: "maya",
        locations: &[("module", "modules/{$name}/")],
        inflect: Inflect::Maya,
    },
    Framework {
        prefix: "mautic",
        locations: &[
            ("plugin", "plugins/{$name}/"),
            ("theme", "themes/{$name}/"),
            ("core", "app/"),
        ],
        inflect: Inflect::Mautic,
    },
    Framework {
        prefix: "matomo",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "mantisbt",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "mako",
        locations: &[("package", "app/packages/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "majima",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Majima,
    },
    Framework {
        prefix: "magento",
        locations: &[
            ("theme", "app/design/frontend/{$name}/"),
            ("skin", "skin/frontend/default/{$name}/"),
            ("library", "lib/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "lms",
        locations: &[
            ("plugin", "plugins/{$name}/"),
            ("template", "templates/{$name}/"),
            ("document-template", "documents/templates/{$name}/"),
            ("userpanel-module", "userpanel/modules/{$name}/"),
        ],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "lithium",
        locations: &[
            ("library", "libraries/{$name}/"),
            ("source", "libraries/_source/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "lavalite",
        locations: &[
            ("package", "packages/{$vendor}/{$name}/"),
            ("theme", "public/themes/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "laravel",
        locations: &[("library", "libraries/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "kohana",
        locations: &[("module", "modules/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "kodicms",
        locations: &[
            ("plugin", "cms/plugins/{$name}/"),
            ("media", "cms/media/vendor/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "known",
        locations: &[
            ("plugin", "IdnoPlugins/{$name}/"),
            ("theme", "Themes/{$name}/"),
            ("console", "ConsolePlugins/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "kanboard",
        locations: &[("plugin", "plugins/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "itop",
        locations: &[("extension", "extensions/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "imagecms",
        locations: &[
            ("template", "templates/{$name}/"),
            ("module", "application/modules/{$name}/"),
            ("library", "application/libraries/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "hurad",
        locations: &[
            ("plugin", "plugins/{$name}/"),
            ("theme", "plugins/{$name}/"),
        ],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "grav",
        locations: &[
            ("plugin", "user/plugins/{$name}/"),
            ("theme", "user/themes/{$name}/"),
        ],
        inflect: Inflect::Grav,
    },
    Framework {
        prefix: "fuelphp",
        locations: &[("component", "components/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "fuel",
        locations: &[
            ("module", "fuel/app/modules/{$name}/"),
            ("package", "fuel/packages/{$name}/"),
            ("theme", "fuel/app/themes/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "fork",
        locations: &[
            ("module", "src/Modules/{$name}/"),
            ("theme", "src/Themes/{$name}/"),
        ],
        inflect: Inflect::ForkCms,
    },
    Framework {
        prefix: "ezplatform",
        locations: &[
            ("meta-assets", "web/assets/ezplatform/"),
            ("assets", "web/assets/ezplatform/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "eliasis",
        locations: &[
            ("component", "components/{$name}/"),
            ("module", "modules/{$name}/"),
            ("plugin", "plugins/{$name}/"),
            ("template", "templates/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "elgg",
        locations: &[("plugin", "mod/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "ee3",
        locations: &[
            ("addon", "system/user/addons/{$name}/"),
            ("theme", "themes/user/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "ee2",
        locations: &[
            ("addon", "system/expressionengine/third_party/{$name}/"),
            ("theme", "themes/third_party/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "drupal",
        locations: &[
            ("core", "core/"),
            ("module", "modules/{$name}/"),
            ("theme", "themes/{$name}/"),
            ("library", "libraries/{$name}/"),
            ("profile", "profiles/{$name}/"),
            (
                "database-driver",
                "drivers/lib/Drupal/Driver/Database/{$name}/",
            ),
            ("drush", "drush/{$name}/"),
            ("custom-theme", "themes/custom/{$name}/"),
            ("custom-module", "modules/custom/{$name}/"),
            ("custom-profile", "profiles/custom/{$name}/"),
            ("drupal-multisite", "sites/{$name}/"),
            ("console", "console/{$name}/"),
            ("console-language", "console/language/{$name}/"),
            ("config", "config/sync/"),
            ("recipe", "recipes/{$name}"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "dolibarr",
        locations: &[("module", "htdocs/custom/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "dokuwiki",
        locations: &[
            ("plugin", "lib/plugins/{$name}/"),
            ("template", "lib/tpl/{$name}/"),
        ],
        inflect: Inflect::DokuWiki,
    },
    Framework {
        prefix: "dframe",
        locations: &[("module", "modules/{$vendor}/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "decibel",
        locations: &[("app", "app/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "croogo",
        locations: &[
            ("plugin", "Plugin/{$name}/"),
            ("theme", "View/Themed/{$name}/"),
        ],
        inflect: Inflect::Croogo,
    },
    Framework {
        prefix: "concretecms",
        locations: &[
            ("core", "concrete/"),
            ("block", "application/blocks/{$name}/"),
            ("package", "packages/{$name}/"),
            ("theme", "application/themes/{$name}/"),
            ("update", "updates/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "concrete5",
        locations: &[
            ("core", "concrete/"),
            ("block", "application/blocks/{$name}/"),
            ("package", "packages/{$name}/"),
            ("theme", "application/themes/{$name}/"),
            ("update", "updates/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "codeigniter",
        locations: &[
            ("library", "application/libraries/{$name}/"),
            ("third-party", "application/third_party/{$name}/"),
            ("module", "application/modules/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "cockpit",
        locations: &[("module", "cockpit/modules/addons/{$name}/")],
        inflect: Inflect::Cockpit,
    },
    Framework {
        prefix: "civicrm",
        locations: &[("ext", "ext/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "chef",
        locations: &[
            ("cookbook", "Chef/{$vendor}/{$name}/"),
            ("role", "Chef/roles/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "ccframework",
        locations: &[
            ("ship", "CCF/orbit/{$name}/"),
            ("theme", "CCF/app/themes/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "cakephp",
        locations: &[("plugin", "Plugin/{$name}/")],
        inflect: Inflect::CakePhp,
    },
    Framework {
        prefix: "botble",
        locations: &[
            ("plugin", "platform/plugins/{$name}/"),
            ("theme", "platform/themes/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "bonefish",
        locations: &[("package", "Packages/{$vendor}/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "bitrix",
        locations: &[
            ("module", "{$bitrix_dir}/modules/{$name}/"),
            ("component", "{$bitrix_dir}/components/{$name}/"),
            ("theme", "{$bitrix_dir}/templates/{$name}/"),
            ("d7-module", "{$bitrix_dir}/modules/{$vendor}.{$name}/"),
            (
                "d7-component",
                "{$bitrix_dir}/components/{$vendor}/{$name}/",
            ),
            ("d7-template", "{$bitrix_dir}/templates/{$vendor}_{$name}/"),
        ],
        inflect: Inflect::Bitrix,
    },
    Framework {
        prefix: "attogram",
        locations: &[("module", "modules/{$name}/")],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "asgard",
        locations: &[("module", "Modules/{$name}/"), ("theme", "Themes/{$name}/")],
        inflect: Inflect::Asgard,
    },
    Framework {
        prefix: "annotatecms",
        locations: &[
            ("module", "addons/modules/{$name}/"),
            ("component", "addons/components/{$name}/"),
            ("service", "addons/services/{$name}/"),
        ],
        inflect: Inflect::Plain,
    },
    Framework {
        prefix: "akaunting",
        locations: &[("module", "modules/{$name}")],
        inflect: Inflect::Camel,
    },
    Framework {
        prefix: "agl",
        locations: &[("module", "More/{$name}/")],
        inflect: Inflect::Agl,
    },
];
