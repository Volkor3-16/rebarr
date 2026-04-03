# Rebarr API Documentation

Complete REST API reference for the Rebarr manga management system.

## Table of Contents

- [Overview](#overview)
- [Authentication](#authentication)
- [Error Handling](#error-handling)
- [Data Types](#data-types)
- [Libraries API](#libraries-api)
- [Manga API](#manga-api)
- [Chapters API](#chapters-api)
- [Settings API](#settings-api)
- [Tasks API](#tasks-api)
- [Import API](#import-api)
- [System API](#system-api)
- [Trusted Groups API](#trusted-groups-api)
- [Webhooks API](#webhooks-api)
- [Provider Scores API](#provider-scores-api)
- [Events API (SSE)](#events-api-sse)

---

## Overview

Rebarr provides a RESTful JSON API for managing manga libraries, chapters, and downloads. All endpoints return JSON responses unless otherwise noted.

**Base URL:** `http://localhost:8000/api`

**Content-Type:** `application/json`

---

## Authentication

Currently, the API does **not** implement authentication. All endpoints are publicly accessible. This is suitable for local/LAN deployments but should be considered when exposing the service externally.

---

## Error Handling

All error responses follow a consistent format:

```json
{
  "error": "Human-readable error message"
}
```

### HTTP Status Codes

| Status | Meaning |
|--------|---------|
| 200 | Success |
| 201 | Created |
| 202 | Accepted (async task queued) |
| 204 | No Content (success, no body) |
| 400 | Bad Request (invalid input) |
| 404 | Not Found |
| 409 | Conflict (duplicate) |
| 500 | Internal Server Error |
| 502 | Bad Gateway (external service failure) |

---

## Data Types

### UUID

All IDs are UUIDs formatted as strings (e.g., `"550e8400-e29b-41d4-a716-446655440000"`).

### Timestamps

All timestamps are Unix timestamps in seconds (i64/i32).

### Enums

#### PublishingStatus
- `"Completed"`
- `"Ongoing"`
- `"Hiatus"`
- `"Cancelled"`
- `"NotYetReleased"`
- `"Unknown"`

#### DownloadStatus
- `"Missing"`
- `"Queued"`
- `"Downloading"`
- `"Downloaded"`
- `"Failed"`

#### MangaSource
- `"AniList"`
- `"Local"`

#### MangaType
- `"Manga"`
- `"Comics"`

#### SynonymSource
- `"AniList"` - Fetched from AniList, can be hidden
- `"Manual"` - User-added, always used for search

---

## Libraries API

### List Libraries

```
GET /api/libraries
```

**Response:** `200 OK`
```json
[
  {
    "uuid": "550e8400-e29b-41d4-a716-446655440000",
    "type": "Manga",
    "root_path": "/data/manga"
  }
]
```

### Create Library

```
POST /api/libraries
```

**Request Body:**
```json
{
  "library_type": "Manga",
  "root_path": "/data/manga"
}
```

**Response:** `201 Created`
```json
{
  "uuid": "550e8400-e29b-41d4-a716-446655440000",
  "type": "Manga",
  "root_path": "/data/manga"
}
```

### Get Library

```
GET /api/libraries/{id}
```

**Response:** `200 OK` (Library object)

### Update Library

```
PUT /api/libraries/{id}
```

**Request Body:**
```json
{
  "root_path": "/new/path/manga"
}
```

**Response:** `200 OK` (Updated Library object)

### Delete Library

```
DELETE /api/libraries/{id}
```

**Response:** `204 No Content`

### List Library Manga

```
GET /api/libraries/{id}/manga
```

**Response:** `200 OK` (Array of Manga objects)

---

## Manga API

### Search AniList

```
GET /api/manga/search?q={query}
```

**Query Parameters:**
- `q` (required): Search query string

**Response:** `200 OK`
```json
[
  {
    "id": "uuid",
    "library_id": "uuid",
    "anilist_id": 12345,
    "mal_id": 67890,
    "metadata": { ... },
    "relative_path": "One Piece",
    "downloaded_count": 100,
    "chapter_count": 1100,
    "metadata_source": "AniList",
    "thumbnail_url": "https://...",
    "monitored": true,
    "created_at": 1700000000,
    "metadata_updated_at": 1700000000,
    "last_checked_at": 1700000000
  }
]
```

### Add Manga (AniList)

```
POST /api/manga
```

**Request Body:**
```json
{
  "anilist_id": 12345,
  "library_id": "550e8400-e29b-41d4-a716-446655440000",
  "relative_path": "One Piece"
}
```

**Response:** `201 Created` (Manga object)

**Errors:**
- `400`: Empty relative_path, invalid library_id
- `404`: Library not found
- `409`: Manga already exists in library
- `502`: AniList lookup failed

### Add Manga (Manual)

```
POST /api/manga/manual
```

**Request Body:**
```json
{
  "library_id": "550e8400-e29b-41d4-a716-446655440000",
  "relative_path": "Custom Manga",
  "title": "My Custom Manga",
  "other_titles": ["Alternative Title"],
  "synopsis": "A great manga...",
  "publishing_status": "Ongoing",
  "tags": ["Action", "Adventure"],
  "start_year": 2020,
  "end_year": null,
  "cover_url": "https://example.com/cover.jpg"
}
```

**Response:** `201 Created` (Manga object)

### Get Manga

```
GET /api/manga/{id}
```

**Response:** `200 OK` (Manga object)

### Delete Manga

```
DELETE /api/manga/{id}
```

**Request Body (optional):**
```json
{
  "delete_files": true
}
```

**Response:** `204 No Content`

### Update Manga

```
PATCH /api/manga/{id}
```

**Request Body:**
```json
{
  "monitored": false
}
```

**Response:** `200 OK` (Updated Manga object)

### List Providers

```
GET /api/providers
```

**Response:** `200 OK`
```json
[
  {
    "name": "MangaDex",
    "needs_browser": false,
    "version": "1.0.0",
    "tags": ["manga"],
    "default_score": 50
  }
]
```

### Scan Manga (Build Chapter List)

```
POST /api/manga/{id}/scan
```

**Response:** `202 Accepted`

### Check New Chapters

```
POST /api/manga/{id}/check-new
```

**Response:** `202 Accepted`

### List Manga Providers

```
GET /api/manga/{id}/providers
```

**Response:** `200 OK`
```json
[
  {
    "provider_name": "MangaDex",
    "provider_url": "https://mangadex.org/title/...",
    "found": true,
    "last_synced_at": 1700000000,
    "search_attempted_at": 1700000000
  }
]
```

### Refresh Metadata

```
POST /api/manga/{id}/refresh
```

**Response:** `202 Accepted`

### Scan Disk

```
POST /api/manga/{id}/scan-disk
```

**Response:** `202 Accepted`

### Serve Cover

```
GET /api/manga/{id}/cover
```

**Response:** `200 OK` (Image file) or `404 Not Found`

### Update Synonyms

```
PATCH /api/manga/{id}/synonyms
```

**Request Body:**
```json
{
  "add": ["New Alternative Title"],
  "hide": ["Unwanted Title"],
  "remove": ["Manual Title To Remove"]
}
```

**Response:** `200 OK` (Updated Manga object)

### Get Provider Candidates

```
GET /api/manga/{id}/providers/{name}/candidates
```

**Response:** `200 OK`
```json
[
  {
    "title": "One Piece",
    "url": "https://mangadex.org/title/...",
    "cover": "https://...",
    "score": 0.95
  }
]
```

### Set Provider URL

```
POST /api/manga/{id}/providers/{name}/url
```

**Request Body:**
```json
{
  "url": "https://mangadex.org/title/..."
}
```
Or to clear:
```json
{
  "url": null
}
```

**Response:** `204 No Content`

### Upload Cover from URL

```
POST /api/manga/{id}/cover
```

**Request Body:**
```json
{
  "url": "https://example.com/cover.jpg"
}
```

**Response:** `200 OK` (Updated Manga object)

### Upload Cover File

```
POST /api/manga/{id}/cover/upload
```

**Request Body:** Raw image data (JPEG, PNG, or WebP, max 10MB)

**Content-Type:** `image/jpeg`, `image/png`, or `image/webp`

**Response:** `200 OK` (Updated Manga object)

---

## Chapters API

### List Chapters

```
GET /api/manga/{id}/chapters
```

**Response:** `200 OK`
```json
[
  {
    "id": "uuid",
    "manga_id": "uuid",
    "chapter_base": 1,
    "chapter_variant": 0,
    "title": "Chapter 1",
    "language": "EN",
    "scanlator_group": "Group Name",
    "provider_name": "MangaDex",
    "chapter_url": "https://...",
    "download_status": "Downloaded",
    "released_at": 1700000000,
    "downloaded_at": 1700000000,
    "scraped_at": 1700000000,
    "is_extra": false,
    "is_canonical": true,
    "tier": 2,
    "file_size_bytes": 10485760
  }
]
```

### Download Chapter

```
POST /api/manga/{id}/chapters/{base}/{variant}/download
```

**Response:** `202 Accepted`

### Delete Chapter

```
DELETE /api/manga/{id}/chapters/{base}/{variant}
```

**Response:** `204 No Content`

### Mark Chapter Downloaded

```
POST /api/manga/{id}/chapters/{base}/{variant}/mark-downloaded
```

**Response:** `204 No Content`

### Reset Chapter

```
POST /api/manga/{id}/chapters/{base}/{variant}/reset
```

**Response:** `204 No Content`

### Toggle Extra Status

```
POST /api/manga/{id}/chapters/{base}/{variant}/toggle-extra
```

**Response:** `204 No Content`

### Optimise Chapter

```
POST /api/manga/{id}/chapters/{base}/{variant}/optimise
```

**Response:** `202 Accepted`

### Set Canonical Chapter

```
POST /api/manga/{id}/chapters/{base}/{variant}/set-canonical
```

**Request Body:**
```json
{
  "chapter_id": "uuid"
}
```

**Response:** `204 No Content`

---

## Settings API

### Get Settings

```
GET /api/settings
```

**Response:** `200 OK`
```json
{
  "scan_interval_hours": 6,
  "queue_paused": false,
  "browser_worker_count": 3,
  "preferred_language": "en",
  "synonym_filter_languages": "zh,vi,ru",
  "wizard_completed": true,
  "default_monitored": true,
  "min_tier": 4,
  "auto_unmonitor_completed": false,
  "download_mode": "must_have"
}
```

### Update Settings

```
PUT /api/settings
```

**Request Body (all fields optional):**
```json
{
  "scan_interval_hours": 12,
  "queue_paused": true,
  "browser_worker_count": 5,
  "preferred_language": "en",
  "synonym_filter_languages": "zh,vi",
  "wizard_completed": true,
  "default_monitored": false,
  "min_tier": 2,
  "auto_unmonitor_completed": true,
  "download_mode": "best_only"
}
```

**Response:** `204 No Content`

**Validation:**
- `scan_interval_hours`: 1-168
- `browser_worker_count`: 1-16
- `min_tier`: 1-4
- `download_mode`: "best_only" or "must_have"

---

## Tasks API

### List Tasks

```
GET /api/tasks?manga_id={uuid}&limit={n}
```

**Query Parameters:**
- `manga_id` (optional): Filter by manga
- `limit` (optional): Limit results (0 = no limit)

**Response:** `200 OK`
```json
[
  {
    "id": "uuid",
    "task_type": "DownloadChapter",
    "status": "Running",
    "manga_id": "uuid",
    "chapter_id": "uuid",
    "priority": 10,
    "attempt": 0,
    "max_attempts": 3,
    "last_error": null,
    "progress": {
      "step": "downloading",
      "label": "Downloading images",
      "current": 50,
      "total": 100,
      "unit": "pages"
    },
    "manga_title": "One Piece",
    "chapter_number_raw": "1050",
    "created_at": "2024-01-01T00:00:00Z",
    "updated_at": "2024-01-01T00:00:00Z"
  }
]
```

### List Tasks Grouped

```
GET /api/tasks/grouped
```

**Response:** `200 OK`
```json
[
  {
    "display_name": "System",
    "is_provider": false,
    "provider_name": null,
    "tasks": [...],
    "running_count": 2,
    "pending_count": 5,
    "total_count": 7,
    "worker_count": 2
  },
  {
    "display_name": "MangaDex",
    "is_provider": true,
    "provider_name": "MangaDex",
    "tasks": [...],
    "running_count": 1,
    "pending_count": 10,
    "total_count": 11,
    "worker_count": 3
  }
]
```

### Cancel Task

```
POST /api/tasks/{id}/cancel
```

**Response:** `204 No Content`

---

## Import API

### Scan Directory

```
POST /api/import/scan
```

**Request Body:**
```json
{
  "source_dir": "/path/to/cbz/files"
}
```

**Response:** `200 OK`
```json
[
  {
    "file_path": "/path/to/chapter.cbz",
    "detected_title": "One Piece",
    "detected_chapter": 1050,
    "suggested_manga_id": "uuid",
    "suggested_manga_title": "One Piece"
  }
]
```

### Execute Imports

```
POST /api/import/execute
```

**Request Body:**
```json
{
  "imports": [
    {
      "file_path": "/path/to/chapter.cbz",
      "manga_id": "uuid",
      "chapter_base": 1050,
      "chapter_variant": 0
    }
  ]
}
```

**Response:** `200 OK`
```json
{
  "imported_count": 5,
  "skipped_count": 1,
  "errors": []
}
```

### Scan Series Directory

```
POST /api/import/series-scan
```

**Request Body:**
```json
{
  "source_dir": "/path/to/series"
}
```

**Response:** `200 OK`
```json
[
  {
    "folder_name": "One Piece",
    "cbz_count": 1050,
    "path": "/path/to/series/One Piece"
  }
]
```

### Execute Series Imports

```
POST /api/import/series-execute
```

**Request Body:**
```json
{
  "imports": [
    {
      "anilist_id": 21,
      "library_id": "uuid",
      "folder_name": "One Piece",
      "path": "/path/to/series/One Piece"
    }
  ],
  "queue_chapter_scan": true
}
```

**Response:** `200 OK`
```json
{
  "created_count": 3,
  "skipped_count": 0,
  "errors": []
}
```

---

## System API

### Get System Info

```
GET /api/system
```

**Response:** `200 OK`
```json
{
  "process_mem_mb": 256,
  "db_manga_count": 150,
  "db_chapter_count": 15000,
  "db_downloaded_count": 12000,
  "tasks_pending": 5,
  "tasks_running": 2,
  "queue_paused": false
}
```

### Get Desktop Health

```
GET /api/system/desktop
```

**Response:** `200 OK`
```json
{
  "xvfb": true,
  "vnc": true,
  "novnc": true
}
```

### Get Version

```
GET /api/version
```

**Response:** `200 OK`
```json
{
  "version": "1.0.0",
  "build_type": "release",
  "git_commit": "abc12345"
}
```

### Get Changelog

```
GET /api/changelog
```

**Response:** `200 OK` (Plain text markdown)

---

## Trusted Groups API

### List Trusted Groups

```
GET /api/trusted-groups
```

**Response:** `200 OK`
```json
["Official Release", "Known Scanner Group"]
```

### Add Trusted Group

```
POST /api/trusted-groups
```

**Request Body:**
```json
{
  "name": "Official Release"
}
```

**Response:** `201 Created`

### Remove Trusted Group

```
DELETE /api/trusted-groups/{name}
```

**Response:** `200 OK`

---

## Webhooks API

### List Webhooks

```
GET /api/webhooks
```

**Response:** `200 OK`
```json
[
  {
    "id": "uuid",
    "target_url": "https://example.com/webhook",
    "enabled": true,
    "task_types": ["DownloadChapter", "RefreshMetadata"],
    "task_statuses": ["Completed", "Failed"],
    "body_template": null,
    "created_at": "2024-01-01T00:00:00Z",
    "updated_at": "2024-01-01T00:00:00Z"
  }
]
```

### Create Webhook

```
POST /api/webhooks
```

**Request Body:**
```json
{
  "target_url": "https://example.com/webhook",
  "enabled": true,
  "task_types": ["DownloadChapter"],
  "task_statuses": ["Completed", "Failed"],
  "body_template": null
}
```

**Valid Task Types:**
- `BuildFullChapterList`
- `RefreshMetadata`
- `CheckNewChapter`
- `DownloadChapter`
- `ScanDisk`
- `OptimiseChapter`
- `Backup`

**Valid Task Statuses:**
- `Pending`
- `Running`
- `Completed`
- `Failed`
- `Cancelled`

**Response:** `201 Created` (Webhook object)

### Update Webhook

```
PUT /api/webhooks/{id}
```

**Request Body:** Same as Create Webhook

**Response:** `200 OK` (Updated Webhook object)

### Delete Webhook

```
DELETE /api/webhooks/{id}
```

**Response:** `204 No Content`

---

## Provider Scores API

### Get Global Score

```
GET /api/providers/{name}/score
```

**Response:** `200 OK`
```json
{
  "score": 75,
  "enabled": true,
  "default_score": 50
}
```

### Set Global Score

```
PUT /api/providers/{name}/score
```

**Request Body:**
```json
{
  "score": 75,
  "enabled": true
}
```

**Response:** `200 OK` (Updated score object)

### Delete Global Score

```
DELETE /api/providers/{name}/score
```

**Response:** `200 OK` (Reverts to default)

### Get Series Score

```
GET /api/manga/{id}/providers/{name}/score
```

**Response:** `200 OK`
```json
{
  "score": 100,
  "enabled": true,
  "effective_score": 100,
  "default_score": 50,
  "score_source": "series"
}
```

### Set Series Score

```
PUT /api/manga/{id}/providers/{name}/score
```

**Request Body:**
```json
{
  "score": 100,
  "enabled": true
}
```

**Response:** `200 OK` (Updated score object)

### Delete Series Score

```
DELETE /api/manga/{id}/providers/{name}/score
```

**Response:** `200 OK` (Reverts to global/default)

---

## Events API (SSE)

Server-Sent Events endpoint for real-time task updates.

### Connect to Events

```
GET /api/events
```

**Response:** `200 OK` (Event Stream)

**Content-Type:** `text/event-stream`

### Event Format

```
data: {"id":"uuid","task_type":"DownloadChapter","status":"Running","manga_title":"One Piece","chapter_number_raw":"1050","last_error":null}

data: ping
```

### Event Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Task UUID |
| `task_type` | string | Task type name |
| `status` | string | Current status |
| `manga_title` | string? | Manga title |
| `chapter_number_raw` | string? | Chapter number (e.g., "1050" or "1050.5") |
| `last_error` | string? | Error message if failed |

### Client Implementation Example

```javascript
const events = new EventSource('/api/events');

events.onmessage = (event) => {
  if (event.data === 'ping') return;
  
  const update = JSON.parse(event.data);
  console.log(`Task ${update.id}: ${update.status}`);
};

events.onerror = (error) => {
  console.error('SSE connection error:', error);
};
```

---

## Appendix: Common Patterns

### Pagination

Currently, most list endpoints do **not** support pagination. The `/api/tasks` endpoint accepts an optional `limit` parameter.

### Filtering

- `/api/tasks` supports `manga_id` filter
- `/api/manga/<id>/chapters` applies `preferred_language` setting automatically

### Async Operations

Endpoints returning `202 Accepted` queue background tasks. Use the SSE endpoint or poll `/api/tasks` to monitor progress.

### File Uploads

Cover uploads accept raw binary data with a 10MB limit. Supported formats: JPEG, PNG, WebP.