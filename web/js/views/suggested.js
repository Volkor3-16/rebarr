import { render } from '../router.js';

export function viewSuggested() {
  render(`
    <section class="settings-card" style="max-width:760px">
      <div class="settings-card-header">
        <iconify-icon icon="mdi:lightbulb-on-outline" width="20" height="20"></iconify-icon>
        <h3>Suggested Manga</h3>
      </div>
      <p class="settings-card-desc">
        This page is a stub for future AniList-powered suggestions and recommendation browsing.
      </p>
      <p>
        Recommendations are not wired up yet, but the route is live so we can start hanging UI and API work off it cleanly.
      </p>
    </section>
  `);
}

window.viewSuggested = viewSuggested;
