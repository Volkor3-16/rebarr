

## Fields We Can't Populate Yet (notes for later)

Writer / Penciller / Inker / Colorist / Letterer / CoverArtist / Editor	AniList has staff data via staff field — not currently fetched. Could add to AniList query.
Publisher / Imprint	AniList doesn't expose this reliably for manga
BlackAndWhite	User preference or tag inference — needs manual entry field
AgeRating	AniList has isAdult bool and tags like "Ecchi" — could map to "M" / "Adults Only 18+"
Month / Day	AniList has startDate.month and startDate.day — currently not fetched
Characters / Teams / Locations	Not in AniList manga API
StoryArc / SeriesGroup	Not tracked
CommunityRating	AniList has averageScore (0-100) — could map to 0-5 scale
Format	Could default to "manga" string
Pages element	Would need image dimensions (currently not tracked)