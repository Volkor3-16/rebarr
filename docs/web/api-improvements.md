# AI Slop frontend API improvements

I want to have a decent working and looking frontend, but I know fuck-all about frontend stuff. We've build the frontend incrementally and almost entirely with AI. So I've asked it to go through and find the problems. (grok make no mistakes plz)

## Stuff that isn't that should

- [ ] No Pagination
  - Larger libraries will use more memory and slower responses.
  - No way for infinite scroll (nicely)
  - Mobile/remote clients will be awfully slow.
  - `GET /api/libraries`
  - `GET /api/libraries/{id}/manga`
  - `GET /api/manga/{id}/chapters`
  - `GET /api/tasks` (has `limit` but no offset)
  - `GET /api/webhooks`
  - `GET /api/trusted-groups`
- [ ] No Sorting Parameters
  - We can't sort by: chapter number, release date, download status, last updated
  - `GET /api/manga/{id}/chapters?sort=released_at&order=desc`
- [ ] No filtering/seach on list endpoints
  - Missing filters: Manga by publishing status, published status, chapters by download/provider/language, tasks by status
  - `GET /api/libraries/{id}/manga?monitored=true&status=Ongoing`
  - `GET /api/manga/{id}/chapters?status=Downloaded&language=EN`
  - `GET /api/tasks?status=Running&type=DownloadChapter`
- [ ] No Bulk Operations
  - Download multiple chapters, delete multiple chapters, mark multiple chapters as downloaded, toggle monitored for multiple series, cancel multiple tasks
  - `POST /api/chapters/bulk/download`
  - `POST /api/chapters/bulk/delete`
  - `POST /api/tasks/bulk/cancel`

## Stuff that's bad and could be better

- [ ] Inconsistent ID Parameter Names
  - Why does everything use `id` but Chapters table uses base + variant?
- [ ] Inconsistent Response Wrapping
  - Some endpoints return the object directly, some don't.
  - GET /api/manga/{id} - returns object directly
  - POST /api/import/execute - returns summary
- [ ] Inconsistent HTTP Methods
  - `POST /api/manga/{id}/providers/{name}/url` - Should be `PUT` for updating a resource
  - `POST /api/manga/{id}/cover` - Should be `PUT` for updating cover URL
- [ ] Missing HTTP Status Codes
  - `DELETE` endpoints return `204 No Content` but some return `200 OK`
  - `POST` for creation should return `201 Created` consistently
  - No `422 Unprocessable Entity` for validation errors


## Missing Endpoints

- [ ] Bulk Chapter Download
  - `POST /api/manga/{id}/chapters/bulk/download`
  - `GET /api/manga?library_id={uuid}&monitored=true&status=Ongoing`
- [ ] Manga Statistics
  - `GET /api/manga/{id}/stats` (total chapters, downloaded, missing, queued,failed)
- [ ] Library Statistics
  - `GET /api/libraries/{id}/stats` (manga_count, total chapters, downloaded chapters, total size)
- [ ] Global Statistics
  - `GET /api/stats` (total manga, total chapter,s downloaded chapters, total size, active downloads, queue depth)
- [ ] Search Across All Manga (not AllManga the provider though)
  - `GET /api/search?q=One+Piece&type=manga`
  - `GET /api/search?q=MangaDex&type=provider`
- [ ] Export/Backup
  - `POST /api/backup/export`
  - `POST /api/backup/import`
- [ ] WebSocket Support?
  - Allows for:
    - Real-time task updates
    - Live download progress with image previews
    - Bidirectional communication

---

## Response Format Improvements

- [ ] Include Related Resources
  - Getting manga doesn't include it's chapters. Frontend must make additional requests
  - add `include` param, `GET /api/manga/{id}?include=chapters,providers`
- [ ] Field Selection
  - Large responses waste bandwidth
  - add `fields` param, `GET /api/libraries/{id}/manga?fields=id,title,thumbnail_url,monitored`
- [ ] Consistent Date Formats
  - Use ISO 8601 instead of unix timestamps?
  - Why? is it easier for frontend?


## Error Handling Improvements

- [ ] Structured Error Responses
  - Current: `{"error": "manga not found"}`
  - Better: `{"error": {"code": "RESOURCE_NOT_FOUND""message": "Manga not found", "details": {"resource_type": "manga","resource_id": "550e8400-e29b-41d4-a716-446655440000"} }}`
- [ ] Validation Error Details
  - Current: Generic error message... yuck
  - Better: Field-level validation errors:
  ```json
  {
    "error": {
      "code": "VALIDATION_ERROR",
      "message": "Invalid request",
      "fields": {
        "root_path": "Path does not exist",
        "library_type": "Must be 'Manga' or 'Comics'"
      }
    }
  }
  ```
- [ ] Rate Limiting Headers
  - Little useless, but could be helpful on larger instances
  - `X-RateLimit-Limit: 100` `X-RateLimit-Remaining: 95` `X-RateLimit-Reset: 1700000060`

- [ ] CORS setup (oh boy)
  - Development origins (localhost)
  - Configurable allowed origins
  - Proper preflight handling

- [ ] Caching Headers
  - Static assets: `Cache-Control: public, max-age=31536000`
  - API responses: `Cache-Control: no-cache` or `ETag` support
  - Cover images: `Cache-Control: public, max-age=86400`

- [ ] Performance Considerations
  - Chapter Lists can be giant
    - Pagination?
    - Gzip?
    - Response streaming????


## Frontend-Specific Recommendations

- [ ] Optimistic Updates
  - API Should support `PATCH` for partial updates to enable optimistic UI updates.
- [ ] Idempotency Keys
  - For critical operations, support idempotency keys to prevent duplicate actions:
  - `POST /api/manga/{id}/chapters/{base}/{variant}/download`
  - `X-Idempotency-Key: unique-request-id`
- [ ] Request Deduplication
  - Document which operations are safe to retry and which aren't.
- [ ] Progress Reporting
  - ```json
    {
      "progress": {
        "phase": "downloading_images",
        "current": 45,
        "total": 100,
        "bytes_downloaded": 52428800,
        "bytes_total": 116469760,
        "speed_bytes_per_sec": 1048576,
        "eta_seconds": 61
      }
    }
    ```

---

## Volkor's Notes

- [ ] POST `/api/manga/<id>/chapters/<base>/<variant>/download`
  - This could surely just be `/api/manga/<id>/chapter/<chapter-uuid>/download` why are we telling it to pick for itself, surely it knows the chapter numbers and can handle it nicely.
- [ ] Same with POST `/api/manga/<id>/chapters/<base>/<variant>/toggle-extra`
  - Why aren't we doing this with chapter uuids?
- This should be done for almost all chapter handling. We shouldn't be relying on what chapter is canonical other than downloading and the frontend view.