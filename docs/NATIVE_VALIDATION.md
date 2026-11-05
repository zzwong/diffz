# Native interaction validation

The text geometry probe does not test pointer dispatch, focus, scrolling, or
accessibility. Use a dev build on Linux or macOS to check these behaviors when
the comment composer or reader interaction code changes.

1. Open a fixture or review, select a changed line, and open a new comment.
   Confirm the editor receives focus and typing creates a locally saved draft.
2. Type `/tab`, move the pointer across the slash results, and use arrow keys
   and Enter. Confirm the highlighted command follows both pointer and keyboard
   input. Use Escape and confirm the menu closes without deleting the draft.
3. Open the table command. Hover cells, use the row and column controls, enter
   dimensions directly, then insert. Confirm the Markdown table has the chosen
   size and that every control visibly responds to hover and press.
4. Select text and apply heading, emphasis, link, quote, code, list, and
   checklist actions. Confirm the selection or caret lands in the useful place.
5. Toggle Preview and verify the rendered Markdown, including a table and code
   block. Return to editing and confirm text, selection, and focus are retained.
6. Save a reply, reopen it from the slash menu, then remove it. Confirm each
   action changes the menu immediately and survives reopening the app.
7. Close and reopen the draft, then switch to another file and back. Confirm
   the comment and its source range persist. Repeat with a multi-line selection.

For GitLab publication, use a disposable merge request with writes explicitly
enabled. Publish a multi-line comment and request changes, then refresh the MR
in both GitLab and Diffz. Confirm the range and reviewer state match. Do not use
the local fake-`glab` tests as evidence of remote API compatibility.

## Rich-view keyboard paging

Open fixture F01 and enable rich prose mode. With the diff focused, press
PageDown twice and PageUp once. Confirm the visible prose moves by about 85%
of the current viewport each time. Resize the window and repeat. At either end,
repeated paging must stay in the same file without overscrolling.

Switch to source mode and confirm rich paging did not move the source position;
PageUp and PageDown should still scroll the source when it is visible. Switch
back to rich mode and confirm its position is retained. Repeat with the find
input focused and with a modal panel open: page keys must not scroll the diff.

