// Library management view

import { libraries } from '../api.js';
import { render } from '../router.js';
import { escape, skeleton } from '../utils.js';

export async function viewLibraries() {
  render(`<div class="libraries">${skeleton(3)}</div>`);
  
  try {
    const libs = await libraries.list();
    
    let libRows = libs.map(lib => {
      const type = lib.type === 'Comics' ? 'Comics' : 'Manga';
      return `
        <tr class="lib-row" id="librow-${lib.uuid}">
          <td>${escape(type)}</td>
          <td>
            <span id="libpath-${lib.uuid}">${escape(lib.root_path)}</span>
            <div class="edit-form hidden" id="libedit-${lib.uuid}">
              <input type="text" id="libinput-${lib.uuid}" value="${escape(lib.root_path)}">
              <button class="btn btn-sm btn-primary" onclick="saveLibrary('${lib.uuid}')">Save</button>
              <button class="btn btn-sm" onclick="cancelEditLibrary('${lib.uuid}')">Cancel</button>
            </div>
          </td>
          <td>
            <button class="btn btn-sm btn-ghost" onclick="editLibrary('${lib.uuid}')">Edit</button>
            <button class="btn btn-sm btn-error btn-outline" onclick="deleteLibrary('${lib.uuid}')">Delete</button>
            <a href="/search?library_id=${lib.uuid}" data-path="/search?library_id=${lib.uuid}" class="btn btn-sm btn-primary btn-outline">Add Manga</a>
          </td>
        </tr>
      `;
    }).join('');

    render(`
      <h2>Libraries</h2>
      ${libs.length > 0 
        ? `<table><thead><tr><th>Type</th><th>Root Path</th><th></th></tr></thead><tbody>${libRows}</tbody></table>`
        : '<p>No libraries yet.</p>'}
      
      <hr>
      <h3>Add Library</h3>
      <form id="add-library-form">
        <label>Type:
          <select id="al-type">
            <option value="Manga">Manga</option>
            <option value="Comics">Comics (Western)</option>
          </select>
        </label>
        <label>Root Path:
          <input type="text" id="al-path" placeholder="/data/manga">
        </label>
        <button type="submit" class="btn btn-primary">+ Add Library</button>
      </form>
      <div id="al-status"></div>
    `);
    
    // Add form handler
    document.getElementById('add-library-form').addEventListener('submit', async (e) => {
      e.preventDefault();
      const type = document.getElementById('al-type').value;
      const path = document.getElementById('al-path').value.trim();
      const status = document.getElementById('al-status');
      
      if (!path) {
        status.innerHTML = '<p class="error">Root path required.</p>';
        return;
      }
      
      try {
        await libraries.create({ library_type: type, root_path: path });
        viewLibraries();
      } catch (err) {
        status.innerHTML = `<p class="error">Error: ${escape(err.message)}</p>`;
      }
    });
  } catch (e) {
    render(`<p class="error">Error: ${escape(e.message)}</p>`);
  }
}

window.editLibrary = function(uuid) {
  document.getElementById(`libpath-${uuid}`).classList.add('hidden');
  document.getElementById(`libedit-${uuid}`).classList.remove('hidden');
  document.getElementById(`libinput-${uuid}`).focus();
};

window.cancelEditLibrary = function(uuid) {
  document.getElementById(`libpath-${uuid}`).classList.remove('hidden');
  document.getElementById(`libedit-${uuid}`).classList.add('hidden');
};

window.saveLibrary = async function(uuid) {
  const newPath = document.getElementById(`libinput-${uuid}`).value.trim();
  if (!newPath) { alert('Root path cannot be empty.'); return; }
  try {
    await libraries.update(uuid, { root_path: newPath });
    viewLibraries();
  } catch(e) {
    alert('Error: ' + e.message);
  }
};

window.deleteLibrary = async function(uuid) {
  if (!confirm('Delete this library and ALL its manga records? (Files on disk are not deleted.)')) return;
  try {
    await libraries.delete(uuid);
    viewLibraries();
  } catch(e) {
    alert('Error: ' + e.message);
  }
};

window.viewLibraries = viewLibraries;
