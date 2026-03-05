This document explains the scraping / provider system.
It's a little complicated, so sit tight.


Overview:

1. Rebarr loads all (valid) provider yamls
2. user clicks around and uses the app
3. when triggered, rebarr searches for the manga on all providers
4. rebarr saves a cached copy of the manga's chapter list (to combine with other providers for final chapter list)
5. when triggered, rebarr downloads each image in a chapter.


To do that, we nee