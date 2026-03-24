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
export const del = (path) => api('DELETE', path);
export const patch = (path, body) => api('PATCH', path, body);

// Library API
export const libraries = {
  list: () => get('/api/libraries'),
  get: (uuid) => get(`/api/libraries/${uuid}`),
  create: (data) => post('/api/libraries', data),
  update: (uuid, data) => put(`/api/libraries/${uuid}`, data),
  delete: (uuid) => del(`/api/libraries/${uuid}`),
  manga: (uuid) => get(`/api/libraries/${uuid}/manga`),
};

// Manga API
export const manga = {
  get: (id) => get(`/api/manga/${id}`),
  create: (data) => post('/api/manga', data),
  createManual: (data) => post('/api/manga/manual', data),
  update: (id, data) => patch(`/api/manga/${id}`, data),
  delete: (id) => del(`/api/manga/${id}`),
  chapters: (id) => get(`/api/manga/${id}/chapters`),
  providers: (id) => get(`/api/manga/${id}/providers`),
  providerCandidates: (id, name) => get(`/api/manga/${id}/providers/${encodeURIComponent(name)}/candidates`),
  setProviderUrl: (id, name, url) => post(`/api/manga/${id}/providers/${encodeURIComponent(name)}/url`, { url }),
  scan: (id) => post(`/api/manga/${id}/scan`, null),
  checkNew: (id) => post(`/api/manga/${id}/check-new`, null),
  scanDisk: (id) => post(`/api/manga/${id}/scan-disk`, null),
  refresh: (id) => post(`/api/manga/${id}/refresh`, null),
  downloadChapter: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/download`, null),
  resetChapter: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/reset`, null),
  deleteChapter: (id, base, variant) => del(`/api/manga/${id}/chapters/${base}/${variant}`),
  toggleExtra: (id, base, variant) => post(`/api/manga/${id}/chapters/${base}/${variant}/toggle-extra`, null),
  setCanonical: (id, base, variant, chapterId) => post(`/api/manga/${id}/chapters/${base}/${variant}/set-canonical`, { chapter_id: chapterId }),
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
  cancel: (id) => post(`/api/tasks/${id}/cancel`, null),
};

// Trusted Groups API
export const trustedGroups = {
  list: () => get('/api/trusted-groups'),
  add: (name) => post('/api/trusted-groups', { name }),
  remove: (name) => del(`/api/trusted-groups/${encodeURIComponent(name)}`),
};

// System info API
export const system = {
  info: () => get('/api/system'),
  desktop: () => get('/api/system/desktop'),
};

// Import API
export const importApi = {
  scan: (source_dir) => post('/api/import/scan', { source_dir }),
  execute: (imports) => post('/api/import/execute', { imports }),
};

// Provider scores API
export const providerScores = {
  getGlobal: (name) => get(`/api/providers/${encodeURIComponent(name)}/score`),
  setGlobal: (name, score, enabled) => put(`/api/providers/${encodeURIComponent(name)}/score`, { score, enabled }),
  getSeries: (mangaId, name) => get(`/api/manga/${mangaId}/providers/${encodeURIComponent(name)}/score`),
  setSeries: (mangaId, name, score, enabled) => put(`/api/manga/${mangaId}/providers/${encodeURIComponent(name)}/score`, { score, enabled }),
};
