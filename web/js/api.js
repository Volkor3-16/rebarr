// API helpers for communicating with the backend

/**
 * Make an API request
 * @param {string} method - HTTP method
 * @param {string} path - API path
 * @param {object|null} body - Request body (optional)
 * @returns {Promise<object>} Response JSON
 */
export async function api(method, path, body) {
  const opts = { 
    method, 
    headers: { 'Content-Type': 'application/json' } 
  };
  
  if (body !== undefined) {
    opts.body = JSON.stringify(body);
  }
  
  const r = await fetch(path, opts);
  
  if (!r.ok) {
    const e = await r.json().catch(() => ({ error: r.statusText }));
    throw new Error(e.error || r.statusText);
  }
  
  // No content responses
  if (r.status === 204 || r.status === 202) return null;
  
  return r.json();
}

// Convenience methods
export const get = (path) => api('GET', path);
export const post = (path, body) => api('POST', path, body);
export const put = (path, body) => api('PUT', path, body);
export const del = (path, body) => api('DELETE', path, body);
export const patch = (path, body) => api('PATCH', path, body);

// Library API
export const libraries = {
  list: () => get('/api/libraries'),
  get: (uuid) => get(`/api/libraries/${uuid}`),
  create: (data) => post('/api/libraries', data),
  update: (uuid, data) => put(`/api/libraries/${uuid}`, data),
  delete: (uuid) => del(`/api/libraries/${uuid}`),
  manga: (uuid) => get(`/api/libraries/${uuid}/manga`),
  suggestions: (uuid) => get(`/api/libraries/${uuid}/suggestions`),
  refreshSuggestions: (uuid) => post(`/api/libraries/${uuid}/suggestions/refresh`, null),
  setSuggestionHidden: (uuid, anilistId, hidden) => patch(`/api/libraries/${uuid}/suggestions/${anilistId}`, { hidden }),
};

// Manga API
export const manga = {
  get: (id) => get(`/api/manga/${id}`),
  create: (data) => post('/api/manga', data),
  createManual: (data) => post('/api/manga/manual', data),
  update: (id, data) => patch(`/api/manga/${id}`, data),
  delete: (id, data) => del(`/api/manga/${id}`, data),
  chapters: (id) => get(`/api/manga/${id}/chapters`),
  providers: (id) => get(`/api/manga/${id}/providers`),
  providerCandidates: (id, name) => get(`/api/manga/${id}/providers/${encodeURIComponent(name)}/candidates`),
  setProviderUrl: (id, name, url, title) => post(`/api/manga/${id}/providers/${encodeURIComponent(name)}/url`, { url, title: title ?? null }),
  syncProvider: (id, name) => post(`/api/manga/${id}/providers/${encodeURIComponent(name)}/sync`, null),
  scan: (id) => post(`/api/manga/${id}/scan`, null),
  checkNew: (id) => post(`/api/manga/${id}/check-new`, null),
  scanDisk: (id) => post(`/api/manga/${id}/scan-disk`, null),
  refresh: (id) => post(`/api/manga/${id}/refresh`, null),
  downloadChapter: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/download`, null),
  downloadChapterNow: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/download-now`, null),
  resetChapter: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/reset`, null),
  deleteChapter: (id, base, variant) => del(`/api/manga/${id}/chapters/${base}/${variant}`),
  deleteChapterEntry: (id, base, variant) => del(`/api/manga/${id}/chapters/${base}/${variant}/entry`),
  toggleExtra: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/toggle-extra`, null),
  addChapterTag: (id, base, variant, tag) => post(`/api/manga/${id}/chapters/${base}/${variant}/tags`, { tag }),
  removeChapterTag: (id, base, variant, tag) => del(`/api/manga/${id}/chapters/${base}/${variant}/tags/${encodeURIComponent(tag)}`),
  setCanonical: (id, base, variant, chapterId) => post(`/api/manga/${id}/chapters/${base}/${variant}/set-canonical`, { chapter_id: chapterId }),
  clearCanonicalOverride: (id, base, variant) => del(`/api/manga/${id}/chapters/${base}/${variant}/canonical-override`),
  markDownloaded: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/mark-downloaded`, null),
  optimise: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/optimise`, null),
  updateSynonyms: (id, data) => patch(`/api/manga/${id}/synonyms`, data),
};

// Search API
export const search = {
  query: (q) => get(`/api/manga/search?q=${encodeURIComponent(q)}`),
};

// Settings API
export const settings = {
  get: () => get('/api/settings'),
  update: (data) => put('/api/settings', data),
};

export const webhooks = {
  list: () => get('/api/webhooks'),
  create: (data) => post('/api/webhooks', data),
  update: (id, data) => put(`/api/webhooks/${id}`, data),
  delete: (id) => del(`/api/webhooks/${id}`),
};

// Providers API
export const providers = {
  list: () => get('/api/providers'),
};

// Tasks API
export const tasks = {
  list: (params = {}) => {
    const query = new URLSearchParams(params).toString();
    return get(`/api/tasks${query ? '?' + query : ''}`);
  },
  listQueue: (params = {}) => {
    const query = new URLSearchParams(params).toString();
    return get(`/api/tasks/queue${query ? '?' + query : ''}`);
  },
  cancel: (id) => post(`/api/tasks/${id}/cancel`, null),
  prioritise: (id) => post(`/api/tasks/${id}/prioritise`, null),
  listGrouped: () => get('/api/tasks/grouped'),
};


// System info API
export const system = {
  info: () => get('/api/system'),
  desktop: () => get('/api/system/desktop'),
  version: () => get('/api/version'),
  changelog: () => fetch('/api/changelog').then(r => r.text()),
  purgeOrphanCbz: () => post('/api/system/purge-orphan-cbz', null),
  pruneTaskRetention: () => post('/api/system/task-retention/prune', null),
  scanDiskAll: () => post('/api/system/scan-disk-all', null),
};

// Import API
export const importApi = {
  scan: (source_dir) => post('/api/import/scan', { source_dir }),
  execute: (imports) => post('/api/import/execute', { imports }),
  seriesScan: (source_dir) => post('/api/import/series-scan', { source_dir }),
  seriesExecute: (data) => post('/api/import/series-execute', data),
};

// Cover API
export const coverApi = {
  uploadUrl: (mangaId, url) => post(`/api/manga/${mangaId}/cover`, { url }),
  uploadFile: async (mangaId, file) => {
    const r = await fetch(`/api/manga/${mangaId}/cover/upload`, {
      method: 'POST',
      body: file,
    });
    if (!r.ok) {
      const e = await r.json().catch(() => ({ error: r.statusText }));
      throw new Error(e.error || r.statusText);
    }
    return r.json();
  },
};

// Metadata Rules API
export const metadataRules = {
  list: () => get('/api/metadata-rules'),
  create: (data) => post('/api/metadata-rules', data),
  update: (id, data) => put(`/api/metadata-rules/${id}`, data),
  delete: (id) => del(`/api/metadata-rules/${id}`),
};

// Quality Rules API
export const qualityRules = {
  list: () => get('/api/quality-rules'),
  fields: () => get('/api/quality-rules/fields'),
  create: (data) => post('/api/quality-rules', data),
  update: (id, data) => put(`/api/quality-rules/${id}`, data),
  delete: (id) => del(`/api/quality-rules/${id}`),
  reorder: (ordering) => post('/api/quality-rules/reorder', { ordering }),
};

// Provider settings API (enable/disable per provider, globally or per-series)
export const providerSettings = {
  getGlobal: (name) => get(`/api/providers/${encodeURIComponent(name)}/settings`),
  setGlobal: (name, enabled) => put(`/api/providers/${encodeURIComponent(name)}/settings`, { enabled }),
  getSeries: (mangaId, name) => get(`/api/manga/${mangaId}/providers/${encodeURIComponent(name)}/settings`),
  setSeries: (mangaId, name, enabled) => put(`/api/manga/${mangaId}/providers/${encodeURIComponent(name)}/settings`, { enabled }),
  deleteSeries: (mangaId, name) => del(`/api/manga/${mangaId}/providers/${encodeURIComponent(name)}/settings`),
};
