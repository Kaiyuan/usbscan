// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

// UsbScan landing page: language loading, platform detection, and release lookup.
(function () {
  'use strict';

  var LANGUAGES = [
    ['zh-Hans', '简体中文'],
    ['zh-Hant', '繁體中文'],
    ['en', 'English'],
    ['de', 'Deutsch'],
    ['fr', 'Français'],
    ['it', 'Italiano'],
    ['es', 'Español'],
    ['ja', '日本語'],
    ['ko', '한국어']
  ];
  var STORAGE_KEY = 'usbscan-lang';
  var DEFAULT_LANG = 'en';
  var dictionaries = Object.create(null);
  var currentLang = DEFAULT_LANG;
  var releaseState = { kind: 'loading' };
  var languageRequest = 0;
  var currentPlatform = 'windows';

  // -------------------------------------------------------------------------
  // 访客操作系统与架构检测
  // -------------------------------------------------------------------------
  function detectOS() {
    var ua = navigator.userAgent || '';
    var platform = navigator.platform || '';
    if (/macintosh|mac os x|mac_powerpc/i.test(ua) || /Mac/i.test(platform)) {
      return 'macos';
    }
    if (/linux|x11/i.test(ua) || /Linux/i.test(platform)) {
      return 'linux';
    }
    return 'windows';
  }

  function isSupported(lang) {
    return LANGUAGES.some(function (item) { return item[0] === lang; });
  }

  function normalizeLang(value) {
    if (!value) return null;
    var tag = String(value).toLowerCase().replace(/_/g, '-');
    if (tag.indexOf('zh') === 0) {
      return /hant|tw|hk|mo/.test(tag) ? 'zh-Hant' : 'zh-Hans';
    }
    var base = tag.split('-')[0];
    if (['en', 'de', 'fr', 'it', 'es', 'ja', 'ko'].indexOf(base) !== -1) return base;
    return null;
  }

  function pickInitialLang() {
    var query = null;
    try { query = new URLSearchParams(location.search).get('lang'); } catch (e) {}
    var saved = null;
    try { saved = localStorage.getItem(STORAGE_KEY); } catch (e) {}
    var candidates = [query, saved].concat(navigator.languages || [navigator.language || 'en']);
    for (var i = 0; i < candidates.length; i++) {
      var lang = normalizeLang(candidates[i]);
      if (lang) return lang;
    }
    return DEFAULT_LANG;
  }

  function dict() {
    return dictionaries[currentLang] || dictionaries[DEFAULT_LANG] || {};
  }

  function setMeta(attr, name, content) {
    if (!content) return;
    var el = document.querySelector('meta[' + attr + '="' + name + '"]');
    if (el) el.setAttribute('content', content);
  }

  function renderReleaseNote() {
    var note = document.getElementById('release-note');
    if (!note) return;
    var d = dict();
    var key = releaseState.kind === 'ready' ? 'download.noteReady'
      : releaseState.kind === 'incomplete' ? 'download.noteIncomplete'
      : releaseState.kind === 'unavailable' ? 'download.noteUnavailable'
      : 'download.noteLoading';
    if (d[key] != null) note.innerHTML = d[key];
  }

  function applyStrings(d) {
    document.documentElement.lang = currentLang;
    var defaultDict = dictionaries[DEFAULT_LANG] || {};

    document.querySelectorAll('[data-i18n]').forEach(function (el) {
      var key = el.getAttribute('data-i18n');
      var value = d[key] != null ? d[key] : defaultDict[key];
      if (value != null) el.textContent = value;
    });
    document.querySelectorAll('[data-i18n-html]').forEach(function (el) {
      var key = el.getAttribute('data-i18n-html');
      var value = d[key] != null ? d[key] : defaultDict[key];
      if (value != null) el.innerHTML = value;
    });
    document.querySelectorAll('[data-i18n-aria]').forEach(function (el) {
      var key = el.getAttribute('data-i18n-aria');
      var value = d[key] != null ? d[key] : defaultDict[key];
      if (value != null) el.setAttribute('aria-label', value);
    });
    document.querySelectorAll('[data-i18n-alt]').forEach(function (el) {
      var key = el.getAttribute('data-i18n-alt');
      var value = d[key] != null ? d[key] : defaultDict[key];
      if (value != null) el.setAttribute('alt', value);
    });

    if (d['page.title'] || defaultDict['page.title']) document.title = d['page.title'] || defaultDict['page.title'];
    setMeta('name', 'description', d['page.description'] || defaultDict['page.description']);
    setMeta('property', 'og:title', d['page.title'] || defaultDict['page.title']);
    setMeta('property', 'og:description', d['page.description'] || defaultDict['page.description']);

    updateHeroButton();
    renderReleaseNote();
  }

  function fetchDictionary(lang) {
    if (dictionaries[lang]) return Promise.resolve(dictionaries[lang]);
    return fetch('i18n/' + lang + '.json').then(function (response) {
      if (!response.ok) throw new Error('HTTP ' + response.status);
      return response.json();
    }).then(function (data) {
      dictionaries[lang] = data;
      return data;
    });
  }

  function loadLanguage(lang, persist) {
    var target = isSupported(lang) ? lang : DEFAULT_LANG;
    var request = ++languageRequest;

    if (target !== DEFAULT_LANG && !dictionaries[DEFAULT_LANG]) {
      fetchDictionary(DEFAULT_LANG).catch(function () {});
    }

    fetchDictionary(target).catch(function (error) {
      if (target === DEFAULT_LANG) throw error;
      return fetchDictionary(DEFAULT_LANG);
    }).then(function (data) {
      if (request !== languageRequest) return;
      currentLang = dictionaries[target] ? target : DEFAULT_LANG;
      applyStrings(data);

      var select = document.getElementById('lang-select');
      if (select) select.value = currentLang;

      if (persist) {
        try { localStorage.setItem(STORAGE_KEY, currentLang); } catch (e) {}
        try {
          var url = new URL(location.href);
          url.searchParams.set('lang', currentLang);
          history.replaceState(null, '', url);
        } catch (e) {}
      }
    }).catch(function () {});
  }

  function buildLanguageSelect() {
    var select = document.getElementById('lang-select');
    if (!select) return;
    LANGUAGES.forEach(function (item) {
      var option = document.createElement('option');
      option.value = item[0];
      option.textContent = item[1];
      select.appendChild(option);
    });
    select.addEventListener('change', function () {
      loadLanguage(select.value, true);
    });
  }

  // -------------------------------------------------------------------------
  // 平台切换与 Hero 按钮自适应
  // -------------------------------------------------------------------------
  function setPlatform(platform) {
    currentPlatform = platform;

    // 更新 Tab 按钮状态
    document.querySelectorAll('.tab-btn').forEach(function (btn) {
      btn.classList.toggle('active', btn.getAttribute('data-platform') === platform);
    });

    // 更新面板状态
    document.querySelectorAll('.platform-content').forEach(function (panel) {
      panel.classList.toggle('active', panel.id === 'panel-' + platform);
    });

    // 更新侧边栏
    var sideSystem = document.getElementById('side-system');
    var sideArch = document.getElementById('side-arch');
    if (platform === 'macos') {
      if (sideSystem) sideSystem.textContent = 'macOS 10.15+';
      if (sideArch) sideArch.textContent = 'Apple Silicon (M1-M4) / Intel';
    } else if (platform === 'linux') {
      if (sideSystem) sideSystem.textContent = 'UOS / Debian / Ubuntu';
      if (sideArch) sideArch.textContent = 'x86_64 / arm64 (aarch64)';
    } else {
      if (sideSystem) sideSystem.textContent = 'Windows 10 / 11';
      if (sideArch) sideArch.textContent = 'x86_64 (x64)';
    }

    updateHeroButton();
  }

  function updateHeroButton() {
    var heroBtn = document.getElementById('hero-download');
    var heroText = document.getElementById('hero-download-text');
    if (!heroBtn || !heroText) return;

    var d = dict();
    if (currentPlatform === 'macos') {
      var macLink = document.getElementById('macos-universal-link');
      if (macLink && macLink.href && macLink.href !== '#') heroBtn.href = macLink.href;
      heroText.textContent = d['hero.downloadMac'] || '下载 macOS 版 (Universal)';
    } else if (currentPlatform === 'linux') {
      var linuxLink = document.getElementById('linux-deb-link');
      if (linuxLink && linuxLink.href && linuxLink.href !== '#') heroBtn.href = linuxLink.href;
      heroText.textContent = d['hero.downloadLinux'] || '下载 UOS / Linux 版 (.deb)';
    } else {
      var winLink = document.getElementById('setup-link');
      if (winLink && winLink.href && winLink.href !== '#') heroBtn.href = winLink.href;
      heroText.textContent = d['hero.downloadWin'] || '下载 Windows 版 (x64)';
    }
  }

  function setupPlatformTabs() {
    document.querySelectorAll('.tab-btn').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var plat = btn.getAttribute('data-platform');
        if (plat) setPlatform(plat);
      });
    });
  }

  function setDownloadHref(id, url) {
    var link = document.getElementById(id);
    if (link && url) link.href = url;
  }

  // -------------------------------------------------------------------------
  // GitHub Releases 匹配与绑定
  // -------------------------------------------------------------------------
  function bindRelease() {
    var fallback = 'https://github.com/Kaiyuan/usbscan/releases/latest';
    ['setup-link', 'portable-link', 'hero-download', 'macos-universal-link',
     'macos-arm64-link', 'macos-x64-link', 'linux-deb-link', 'linux-deb-arm64-link',
     'linux-tar-link'].forEach(function (id) {
      setDownloadHref(id, fallback);
    });

    var api = 'https://api.github.com/repos/Kaiyuan/usbscan/releases/latest';
    fetch(api, { headers: { Accept: 'application/vnd.github+json' } })
      .then(function (response) {
        if (!response.ok) throw new Error('no release');
        return response.json();
      })
      .then(function (release) {
        var versionEl = document.getElementById('latest-version');
        if (versionEl) versionEl.textContent = release.tag_name || '—';
        var assets = release.assets || [];

        // Windows
        var setup = assets.find(function (a) { return /setup.*\.exe$/i.test(a.name); });
        var portable = assets.find(function (a) { return /(win.*x64|portable).*\.zip$/i.test(a.name); });
        if (setup) setDownloadHref('setup-link', setup.browser_download_url);
        if (portable) setDownloadHref('portable-link', portable.browser_download_url);

        // macOS
        var macUniversal = assets.find(function (a) { return /macos.*universal.*\.tar\.gz$/i.test(a.name); });
        var macArm64 = assets.find(function (a) { return /macos.*(arm64|aarch64).*\.tar\.gz$/i.test(a.name); });
        var macX64 = assets.find(function (a) { return /macos.*x64.*\.tar\.gz$/i.test(a.name); });
        var macAny = macUniversal || assets.find(function (a) { return /macos.*\.tar\.gz$/i.test(a.name); });

        if (macUniversal) setDownloadHref('macos-universal-link', macUniversal.browser_download_url);
        else if (macAny) setDownloadHref('macos-universal-link', macAny.browser_download_url);

        if (macArm64) setDownloadHref('macos-arm64-link', macArm64.browser_download_url);
        if (macX64) setDownloadHref('macos-x64-link', macX64.browser_download_url);

        // Linux / UOS
        var debX64 = assets.find(function (a) { return /(amd64|x64|x86_64).*\.deb$/i.test(a.name); });
        var debArm64 = assets.find(function (a) { return /(arm64|aarch64).*\.deb$/i.test(a.name); });
        var tarX64 = assets.find(function (a) { return /linux.*x64.*\.tar\.gz$/i.test(a.name); });

        if (debX64) setDownloadHref('linux-deb-link', debX64.browser_download_url);
        if (debArm64) setDownloadHref('linux-deb-arm64-link', debArm64.browser_download_url);
        if (tarX64) setDownloadHref('linux-tar-link', tarX64.browser_download_url);

        // Checksums
        var checksums = assets.find(function (a) { return /SHA256SUMS\.txt$/i.test(a.name); });
        if (checksums) setDownloadHref('checksums-link', checksums.browser_download_url);

        releaseState = { kind: assets.length > 0 ? 'ready' : 'incomplete' };
        renderReleaseNote();
        updateHeroButton();
      })
      .catch(function () {
        var versionEl = document.getElementById('latest-version');
        if (versionEl) versionEl.textContent = '—';
        releaseState = { kind: 'unavailable' };
        renderReleaseNote();
      });
  }

  // 初始化
  buildLanguageSelect();
  setupPlatformTabs();
  setPlatform(detectOS());
  loadLanguage(pickInitialLang(), false);
  bindRelease();
})();
