// Home view - shows all manga across all libraries

import { libraries, manga as mangaApi } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton, emptyState } from '../utils.js';

export async function viewHome() {
  render(`<div class="home">${skeleton(5)}</div>`);
  
  try {
    const libs = await libraries.list();
    
    if (libs.length === 0) {
      render(`
        <div class="welcome">
          <h2>Welcome to REBARR</h2>
          <p>No libraries configured yet.</p>
          <button onclick="navigate('/library')">Add a Library</button>
        </div>
      `);
      return;
    }
    
    const mangaLists = await Promise.all(libs.map(lib => libraries.manga(lib.uuid)));
    
    let html = '';
    libs.forEach((lib, i) => {
      const mangas = mangaLists[i];
      const type = lib.type === 'Comics' ? 'Comics' : 'Manga';
      
      html += `<section class="library-section mt-3">`;
      html += `<h3>${escape(lib.root_path)} <small>[${type}]</small></h3>`;
      
      if (mangas.length === 0) {
        html += `<p><small>No manga yet. <a onclick="navigate('/search?library_id=${lib.uuid}')">Add some!</a></small></p>`;
      } else {
        // Card grid view
        html += `<div class="card-grid">`;
        html += mangas.map(m => {
          const dl = m.downloaded_count ?? 0;
          const total = m.chapter_count != null ? m.chapter_count : '?';
          const title = m.metadata?.title ?? 'Unknown';
          const thumb = m.thumbnail_url 
            ? `<img src="${escape(m.thumbnail_url)}" alt="${escape(title)}" loading="lazy">`
            : `<div class="skeleton" style="aspect-ratio: 2/3"></div>`;
          
          return `
            <div class="manga-card" onclick="navigate('/series/${m.id}')">
              ${thumb}
              <div class="info">
                <div class="title">${escape(title)}</div>
                <div class="meta">${dl} / ${total} chapters</div>
              </div>
            </div>
          `;
        }).join('');
        html += `</div>`;
      }
      html += `</section>`;
    });
    
    render(`<div class="home">${html}</div>`);
  } catch (e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

// Make viewHome available for router
window.viewHome = viewHome;
