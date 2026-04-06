Read here for answers.


## Why does the first search take so long?!?!1

Well.. a lot of stuff happens on initial search

tl;dr is The more aliases and the more providers, the longer the scan.

Before running the full scan, you should:
- Make sure you've disabled providers you know don't have it
- Make sure you've disabled aliases that aren't valid (why search for the mandarin name for a english translated japanese comic??)
- Smoke a joint and have some patience. We only do a full scan once a month.

## I can't see my manga in the search!!

You should click the button that says "manual", and enter all the details in you can.

## The provider found a match, but it's not right!

On the series page, you can 'pick' / manually override the match. Scroll down to the providers list, and click **Pick**, it automatically search the provider and return all matches, allowing you to investigate which is right.

Additionally, if you're already on the series on the provider, you can just enter the correct series URL.
Be careful though, if the url isn't correct it will break.

## Some providers don't work! (Cloudflare)

Cloudflare is a bitch. I've gone to great lengths (vibecoding) to work around it.
We do some fancy stealth browser hacks to pretend to not be a browser controlled by a robot, AND if we still get triggered, we just click vaguely around the checkbox until cloudflare passes (or fails)

If it fails, there's not much else we can do, make a issue and fingers crossed someone knows whats up.


## I've edited some providers, but they get deleted when I restart rebarr wtf??

You've run into a problem that I'm not exactly happy with.

Every time rebarr starts, we replace the providers that exist on disk with the stock providers.

If you want to have your own provider, just make sure it doesn't have the same name of one that exists. You can always disable one globally in the settings.

Sorry.

## I've imported some chapters, but after running 'Search All Providers', they're all replaced!

This issue is a little complicated.

1. You import chapters, match the scanlators and import it. Internally, the provider is set to `Local`.
2. You then search all providers, and new (better) chapters are found. Rebarr sees this, and puts them as the new canonical chapter.
3. Rebarr then auto-queues the upgrade system to download them.

If you don't want rebarr to replace your existing chapters, theres a setting that you can enable that disables chapter upgrades. New chapters will still download, but any chapter you already have downloaded will stay canonical until you replace it manually.

The downside of this is if the first release of a chapter is awful, you'll have to open rebarr, delete it, and then search for a replacement.
Other versions will still be found, but they'll never be canonicalised (set as the primary source)

If you want to undo that and let chapters upgrade again, just uncheck the setting and `Check for new Chapters` on each series you want.
(or wait 6ish hours for the auto-check)
