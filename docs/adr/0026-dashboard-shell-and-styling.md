# ADR-0026: Dashboard shell and styling

Status: accepted (fifth nav page added by ADR-0027)

## Context
Issue #35 redesigns the dashboard. The four pages each carried their own copy of
the head and the links, and those copies had drifted. The pages had no
stylesheet. Three of them reloaded with `<meta http-equiv="refresh">`, which
fails WCAG F41 because it reloads the page under the user's focus.

## Decision
- One `app.css`, embedded with `include_str!` and inlined in a `<style>` element
  by an askama `base.html` that every page extends. A versioned external file is
  rejected: for our own CSS it risks stale `immutable` caches, and it needs a
  route, a content hash and a path in every template. It saves almost nothing
  for about 5 to 7 KB on a LAN page that reloads every 60 s.
- The base template owns the head, the skip link, the nav and the page header,
  so the shell cannot drift between pages.
- An inline script replaces the meta refresh. It reads the page's period from a
  `data-refresh` attribute (Overview and Alerts 60 s, Health 300 s) and
  reloads only while the page is visible and no form field has focus, so a
  focused link or table does not stop it. When a hidden page becomes visible
  and a full period has passed since load, it reloads at once. History does
  not reload.
- The overview reads the alerter concurrently with the store. An alerter that
  is down gives status 200 and a notice. A store that is down renders the shell
  and a notice, and keeps status 502, on every page.

## Consequences
- Every page except History carries a small inline script.
- Revisit when the CSS grows past about 20 KB or a CSP without `unsafe-inline`
  is introduced: move to a content-hashed external file. The History chart
  script and the refresh script would then have to move too.
