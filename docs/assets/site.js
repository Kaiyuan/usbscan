// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 usbscan contributors

// UsbScan landing page: language loading, string rendering and release lookup.
// Each language lives in its own JSON file under `i18n/` and is fetched on demand.
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

    document.querySelectorAll('[data-i18n]').forEach(function (el) {
      var value = d[el.getAttribute('data-i18n')];
      if (value != null) el.textContent = value;
    });
    document.querySelectorAll('[data-i18n-aria]').forEach(function (el) {
      var value = d[el.getAttribute('data-i18n-aria')];
      if (value != null) el.setAttribute('aria-label', value);
    });
    document.querySelectorAll('[data-i18n-alt]').forEach(function (el) {
      var value = d[el.getAttribute('data-i18n-alt')];
      if (value != null) el.setAttribute('alt', value);
    });

    if (d['page.title']) document.title = d['page.title'];
    setMeta('name', 'description', d['page.description']);
    setMeta('property', 'og:title', d['page.title']);
    setMeta('property', 'og:description', d['page.description']);

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
    }).catch(function () {
      // Keep the server-rendered default text if translations cannot be loaded.
    });
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

  function bindRelease() {
    var api = 'https://api.github.com/repos/Kaiyuan/usbscan/releases/latest';
    fetch(api, { headers: { Accept: 'application/vnd.github+json' } })
      .then(function (response) {
        if (!response.ok) throw new Error('no release');
        return response.json();
      })
      .then(function (release) {
        document.getElementById('latest-version').textContent = release.tag_name || '—';
        var assets = release.assets || [];
        var setup = assets.find(function (a) { return /setup.*\.exe$/i.test(a.name); });
        var portable = assets.find(function (a) { return /portable.*\.zip$/i.test(a.name); });
        var checksums = assets.find(function (a) { return /SHA256SUMS\.txt$/i.test(a.name); });
        if (setup) document.getElementById('setup-link').href = setup.browser_download_url;
        if (portable) document.getElementById('portable-link').href = portable.browser_download_url;
        if (checksums) document.getElementById('checksums-link').href = checksums.browser_download_url;
        releaseState = { kind: setup && portable ? 'ready' : 'incomplete' };
        renderReleaseNote();
      })
      .catch(function () {
        document.getElementById('latest-version').textContent = '—';
        releaseState = { kind: 'unavailable' };
        renderReleaseNote();
      });
  }

  buildLanguageSelect();
  loadLanguage(pickInitialLang(), false);
  bindRelease();
})();
