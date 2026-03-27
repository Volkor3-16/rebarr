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
If you do hit cloudflare checks, theres only one thing we can try.

1. Open two tabs, on one, pick a series and go down to a provider that's getting blocked, on the other, go into the Desktop page and unlock the mouse/keyboard
2. Click 'Pick' on the blocked provider, and quickly (within 20s) go back and click the cloudflare checkbox.
    - I have some code that tries to click this, but it's mostly untested and who knows how well it works.
3. If you see search results in the pick dialog, it's worked, and we should have no problems for an unknown amount of time.
    - Seriously, how long does the cloudflare session last? idk im just happy i got it this far.

## I've edited some providers, but they get deleted when I restart rebarr wtf??

You've run into a problem that I'm not exactly happy with.

Every time rebarr starts, we replace the providers that exist on disk with the stock providers.

If you want to have your own provider, just make sure it doesn't have the same name of one that exists. You can always disable one globally in the settings.

Sorry.