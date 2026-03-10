# Adding trusted scanlators

until I can be bothered having it handle scanlator list properly

1. find `20260309000002_trusted_groups.sql`
2. Add your trusted scanlator into the insert query.
3. make sure it runs
4. make a PR?

## How tf we do this properly?

- Text list of scanlators, load that?
    - Annoying to add new scanlators
- DB loads list, imports on run?
    - Allows us to add scanlators from web
    - but we have to handle api and all the shit for that.
- Load up a list from somewhere at runtime?
    - is there even a decent community list available?

Ideally, users should be able to easily add their own trusted scan groups.
but honestly the differences between t2 and t3 groups aren't too important imo