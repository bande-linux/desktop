// Second line of defence behind the removals in patches/: anything left that could phone home stays off.
// Telemetry and data reporting
pref("toolkit.telemetry.enabled", false, locked);
pref("toolkit.telemetry.unified", false, locked);
pref("toolkit.telemetry.server", "data:,", locked);
pref("toolkit.telemetry.archive.enabled", false, locked);
pref("toolkit.telemetry.newProfilePing.enabled", false, locked);
pref("toolkit.telemetry.shutdownPingSender.enabled", false, locked);
pref("toolkit.telemetry.updatePing.enabled", false, locked);
pref("toolkit.telemetry.bhrPing.enabled", false, locked);
pref("toolkit.telemetry.firstShutdownPing.enabled", false, locked);
pref("toolkit.coverage.opt-out", true, locked);
pref("datareporting.healthreport.uploadEnabled", false, locked);
pref("datareporting.policy.dataSubmissionEnabled", false, locked);
pref("datareporting.policy.dataSubmissionPolicyBypassNotification", true, locked);
pref("datareporting.usage.uploadEnabled", false, locked);
pref("browser.ping-centre.telemetry", false, locked);
pref("browser.newtabpage.activity-stream.feeds.telemetry", false, locked);
pref("browser.newtabpage.activity-stream.telemetry", false, locked);
pref("dom.security.unexpected_system_load_telemetry_enabled", false, locked);
// Experiments and studies
pref("app.normandy.enabled", false, locked);
pref("app.normandy.api_url", "", locked);
pref("app.shield.optoutstudies.enabled", false, locked);
pref("messaging-system.rsexperimentloader.enabled", false, locked);
pref("nimbus.rollouts.enabled", false, locked);
pref("nimbus.telemetry.targetingContextEnabled", false, locked);
pref("browser.preferences.experimental", false, locked);
pref("browser.preferences.experimental.hidden", true, locked);
pref("browser.preferences.moreFromMozilla", false, locked);
// Crash reports
pref("breakpad.reportURL", "", locked);
pref("browser.tabs.crashReporting.sendReport", false, locked);
pref("browser.crashReports.unsubmittedCheck.enabled", false, locked);
pref("browser.crashReports.unsubmittedCheck.autoSubmit2", false, locked);
// New tab: no sponsored content, stories, weather or promos
pref("browser.newtabpage.activity-stream.showSponsored", false, locked);
pref("browser.newtabpage.activity-stream.system.showSponsored", false, locked);
pref("browser.newtabpage.activity-stream.showSponsoredTopSites", false, locked);
pref("browser.newtabpage.activity-stream.showSponsoredCheckboxes", false, locked);
pref("browser.newtabpage.activity-stream.feeds.section.topstories", false, locked);
pref("browser.newtabpage.activity-stream.feeds.system.topstories", false, locked);
pref("browser.newtabpage.activity-stream.feeds.weatherfeed", false, locked);
pref("browser.newtabpage.activity-stream.feeds.adsfeed", false, locked);
pref("browser.newtabpage.activity-stream.discoverystream.enabled", false, locked);
pref("browser.newtabpage.activity-stream.discoverystream.sections.contextualAds.enabled", false, locked);
pref("browser.newtabpage.activity-stream.discoverystream.merino-feed-experiment", false, locked);
pref("browser.newtabpage.activity-stream.asrouter.providers.cfr", "null", locked);
pref("browser.newtabpage.activity-stream.asrouter.providers.message-groups", "null", locked);
pref("browser.newtabpage.activity-stream.asrouter.providers.messaging-experiments", "null", locked);
pref("browser.newtabpage.activity-stream.asrouter.providers.whats-new-panel", "null", locked);
pref("browser.newtabpage.activity-stream.asrouter.userprefs.cfr.addons", false, locked);
pref("browser.newtabpage.activity-stream.asrouter.userprefs.cfr.features", false, locked);
pref("browser.topsites.contile.enabled", false, locked);
pref("browser.topsites.useRemoteSetting", false, locked);
pref("browser.urlbar.sponsoredTopSites", false, locked);
pref("browser.vpn_promo.enabled", false, locked);
pref("browser.ipProtection.enabled", false, locked);
pref("browser.contentblocking.report.hide_vpn_banner", true, locked);
pref("browser.contentblocking.report.show_mobile_app", false, locked);
pref("browser.contentblocking.report.lockwise.enabled", false, locked);
pref("browser.uitour.enabled", false, locked);
pref("browser.uitour.url", "", locked);
pref("browser.aboutwelcome.enabled", false, locked);
pref("startup.homepage_override_nimbus_disable_wnp", true, locked);
pref("browser.startup.homepage_override.mstone", "ignore");
pref("startup.homepage_welcome_url", "");
pref("startup.homepage_override_url", "");
// Firefox Suggest
pref("browser.urlbar.suggest.quicksuggest.sponsored", false, locked);
pref("browser.urlbar.suggest.quicksuggest.nonsponsored", false, locked);
pref("browser.urlbar.suggest.quicksuggest.all", false, locked);
pref("browser.urlbar.quicksuggest.enabled", false, locked);
pref("browser.urlbar.quicksuggest.online.enabled", false, locked);
pref("browser.urlbar.quicksuggest.mlEnabled", false, locked);
pref("browser.urlbar.merino.endpointURL", "", locked);
pref("browser.urlbar.addons.featureGate", false, locked);
pref("browser.urlbar.mdn.featureGate", false, locked);
pref("browser.urlbar.trending.featureGate", false, locked);
pref("browser.urlbar.weather.featureGate", false, locked);
pref("browser.urlbar.yelp.featureGate", false, locked);
pref("browser.urlbar.market.featureGate", false, locked);
pref("browser.urlbar.importantDates.featureGate", false, locked);
// No default search engine until the first-launch choice
pref("keyword.enabled", false);
pref("browser.search.update", false, locked);
pref("browser.search.suggest.enabled", false);
pref("browser.urlbar.suggest.searches", false);
pref("browser.search.serpEventTelemetryCategorization.enabled", false, locked);
pref("browser.search.region", "US");
pref("browser.region.update.enabled", false, locked);
pref("browser.region.network.url", "", locked);
// Default-browser prompts
pref("browser.shell.checkDefaultBrowser", false, locked);
pref("browser.shell.didSkipDefaultBrowserCheckOnFirstRun", true);
// Updates come from apt
pref("app.update.enabled", false, locked);
pref("app.update.auto", false, locked);
pref("app.update.staging.enabled", false, locked);
pref("app.update.langpack.enabled", false, locked);
pref("extensions.systemAddon.update.enabled", false, locked);
pref("extensions.systemAddon.update.url", "", locked);
pref("media.gmp-gmpopenh264.enabled", false);
// DRM video: nothing downloads and nothing connects until the user turns this on
pref("media.eme.enabled", false);
// Remote settings: nothing fetched, only the data shipped in each update; translation, once on, fetches its models
pref("bande.services.settings.allowedCollectionsFromDump", "main/tracking-protection-lists,security-state/*,main/content-classifier-lists,main/change-password-urls,main/webcompat-interventions,main/addons-data-leak-blocker-domains,main/mfcdm-origins-list,main/url-classifier-exceptions,main/ml-model-allow-deny-list,main/third-party-cookie-blocking-exempt-urls,main/backup-common-passwords-list,main/bounce-tracking-protection-exceptions,main/fingerprinting-protection-overrides,main/password-rules,main/websites-with-shared-credential-backends,main/password-recipes,main/partitioning-exempt-urls,blocklists/addons-bloomfilters,main/tracking-protection-lists,main/anti-tracking-url-decoration,main/hijack-blocklists,main/fxmonitor-breaches,main/language-dictionaries,blocklists/gfx,blocklists/addons,blocklists/plugins,main/moz-essential-domain-fallbacks,main/url-parser-default-unknown-schemes-interventions,main/urlbar-persisted-search-terms,main/devtools-devices,main/devtools-compatibility-browsers,main/ms-images,main/tippytop,main/search-config-icons,main/search-config-v2,main/search-config-overrides-v2,main/search-default-override-allowlist,main/translations-models-v2,main/translations-wasm-v2", locked);
pref("security.remote_settings.crlite_filters.enabled", false);
pref("security.remote_settings.intermediates.enabled", false);
// Safe Browsing: no calls to Google. Tracking Protection stays, on the lists shipped in each update
pref("browser.safebrowsing.malware.enabled", false, locked);
pref("browser.safebrowsing.phishing.enabled", false, locked);
pref("browser.safebrowsing.blockedURIs.enabled", false, locked);
pref("browser.safebrowsing.downloads.enabled", false, locked);
pref("browser.safebrowsing.downloads.remote.enabled", false, locked);
pref("browser.safebrowsing.downloads.remote.url", "", locked);
pref("browser.safebrowsing.provider.mozilla.gethashURL", "", locked);
// Unsolicited network checks and speculation
// Sites may not use push; the connection opens only for a signed-in account
pref("dom.push.enabled", false);
pref("dom.push.connection.enabled", false);
pref("network.captive-portal-service.enabled", false, locked);
pref("captivedetect.canonicalURL", "", locked);
pref("network.connectivity-service.enabled", false, locked);
pref("network.predictor.enabled", false);
pref("network.prefetch-next", false);
pref("network.dns.disablePrefetch", true);
pref("browser.urlbar.speculativeConnect.enabled", false);
pref("browser.places.speculativeConnect.enabled", false);
pref("geo.provider.network.url", "", locked);
pref("geo.provider.use_geoclue", false);
pref("doh-rollout.enabled", false, locked);
pref("doh-rollout.uri", "", locked);
pref("network.trr.mode", 5);
pref("network.trr.confirmationNS", "skip");
pref("browser.send_pings", false, locked);
// Add-on promos; add-on updates stay for what the user installs
pref("extensions.getAddons.cache.enabled", false, locked);
pref("extensions.getAddons.get.url", "", locked);
pref("extensions.getAddons.discovery.api_url", "", locked);
pref("extensions.getAddons.browserMappings.url", "", locked);
pref("extensions.getAddons.langpacks.url", "", locked);
pref("extensions.getAddons.showPane", false, locked);
pref("extensions.htmlaboutaddons.recommendations.enabled", false, locked);
pref("extensions.webservice.discoverURL", "", locked);
pref("browser.discovery.enabled", false, locked);
pref("extensions.webcompat-reporter.enabled", false, locked);
pref("extensions.unsignedScopes", 12);
pref("extensions.autoDisableScopes", 3);
// Translation: off until turned on in Settings
pref("browser.translations.enable", false);
pref("browser.translations.select.enable", false);
pref("browser.translations.quickAction.enabled", false);
pref("browser.ai.control.translations", "available", locked);
// AI features
pref("browser.ml.enable", false, locked);
pref("browser.ml.chat.enabled", false, locked);
pref("browser.ml.chat.menu", false, locked);
pref("browser.ml.chat.page", false, locked);
pref("browser.ml.chat.shortcuts", false, locked);
pref("browser.ml.linkPreview.enabled", false, locked);
pref("browser.tabs.groups.smart.enabled", false, locked);
pref("browser.ai.control.default", "blocked", locked);
pref("extensions.ml.enabled", false, locked);
pref("pdfjs.enableAltText", false);
pref("pdfjs.enableGuessAltText", false);
// Accounts: stay out of the way until asked for
pref("identity.fxaccounts.toolbar.enabled", false);
// Debian
pref("distribution.searchplugins.defaultLocale", "en-US");
pref("spellchecker.dictionary_path", "/usr/share/hunspell");
// Bande: portal seams; tabs share the row with the control well the compositor draws
pref("widget.use-xdg-desktop-portal.file-picker", 1);
pref("widget.use-xdg-desktop-portal.mime-handler", 1);
pref("widget.use-xdg-desktop-portal.settings", 1);
pref("widget.use-xdg-desktop-portal.open-uri", 1);
pref("browser.tabs.inTitlebar", 1);
pref("browser.compactmode.show", true);
pref("signon.management.page.breach-alerts.enabled", false, locked);
pref("signon.rememberSignons", true);
