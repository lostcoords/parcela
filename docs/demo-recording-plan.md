# Record the Parcela product demo

Draft, 4 November 2026. Target: an 80-second, narrated recording for the landing page.
Software features and prices were checked against official sources on this date in Asia/Tokyo. This workflow has not been rehearsed or recorded yet.

Show one real shop running in two worktrees. Start each app with `parcela up`, let Claude and Codex make different visible changes, then use Parcela's side tab to move between them.

Use Parcela's optional in-app widget, called the side tab here. Its documented behavior is in [the CLI guide](site/cli.md#ui).

## Choose software without recurring fees

Constraints: no subscriptions, Cap has not been purchased, and recording and editing should stay in Cap. Voice selection remains open until a short audition succeeds.

[Cap Desktop](https://cap.so/pricing) is the recorder candidate. Its current lifetime desktop option is $58 and includes commercial usage rights. Validate the recording and editing workflow before buying it.

Cap's [Studio Mode guide](https://cap.so/docs/recording/studio-mode) documents cuts, zooms, imported WAV and MP3 audio tracks, and caption editing. A second video editor is unnecessary for the planned workflow. These are documented capabilities, not a hands-on test of the installed version.

[Kokoro](https://github.com/hexgrad/kokoro) is an audition candidate for local AI narration. Its [model card](https://huggingface.co/hexgrad/Kokoro-82M) lists Apache-2.0 weights. Free local inference does not establish that its delivery suits the landing page. Choose the voice after hearing the actual Parcela script.

Record a short silent rehearsal and test importing one narration clip before committing to the full film. Keep the recording, edit, and final export local.

## Review firsthand recording examples

These Reddit accounts are useful for their workflows. Their software choices are examples, not additional purchases for this project.

- [Cap user describing the Pagecord demo](https://www.reddit.com/r/SideProject/comments/1ubsop5/comment/oszaf3t/): the creator says they used Cap to record a walkthrough and planned to rehearse more for the next version. Their linked home page has since changed, so the original video was not verified there.
- [One-minute demo workflow](https://www.reddit.com/r/SideProject/comments/1ubsop5/comment/osypsvd/): a creator describes writing the script, capturing the product, editing, and adding narration separately. Their post points to InsiderSignals as the finished example; that example has not been reviewed here.
- [Terminal-app creator's recording account](https://www.reddit.com/r/SideProject/comments/1ubsop5/how_do_you_make_demo_videos_for_your_products/): the TermRover creator links [their finished demo](https://youtu.be/unAIzaDyIU8) and describes recording many short takes so individual sections were easier to replace. The linked video has not been watched here.
- [Kokoro comparison with audio samples](https://www.reddit.com/r/TextToSpeech/comments/1udgi9s/open_source_tts_comparison_kokoro_82m_vs/): the author links [WAV samples and source code](https://github.com/gauravvij/kokoro-tts-vs-supertonic-3-tts). Commenters dispute the strongest quality claims, so use the samples rather than the headline as evidence.
- [Kokoro cadence criticism](https://www.reddit.com/r/LocalLLaMA/comments/1kfdbdi/is_elevenlabs_still_unbeatable_for_tts_or_good/): a user praises consistency but finds the cadence unnatural during longer listening. That makes a short test with our own script more useful than a blanket recommendation.

## Prepare the demo app

Use a disposable demo project with local sample products. Keep both worktrees on the same starting version.

1. Configure the project with `parcela.yaml` and rehearse startup once.
2. Prepare two worktrees named `sale-banner` and `catalog-layout`.
3. Use separate sample data bindings if the app writes data. Worktree URLs do not separate external databases.
4. Include the optional widget script in both worktrees. Run `parcela ui` to get the dashboard origin, then use that origin for `/widget.js` as described in the CLI guide.
5. Confirm that the side tab opens, marks the current environment, and navigates to the other ready app.
6. Put the shop at the service's root URL for this take. The switcher opens the selected service's root; do not plan a shot around preserving `/shop` automatically.
7. Confirm local HTTPS works before recording, so the take has no certificate or setup prompts.
8. Rehearse each agent task and check the final result. Prepare the worktrees in their unchanged starting state for the real take.

Keep both apps loopback-only. Phone and remote sharing are a separate demonstration.

Give the agents different tasks that read clearly on video:

- Claude: "Add a blue sale banner above the catalog that says 'Summer sale · 20% off'. Keep the product grid unchanged. Work only in this checkout."
- Codex: "Change the catalog into a clean list with an image, name, and price on each row. Keep the page heading and colors unchanged. Work only in this checkout."

These tasks are more visible than a small price edit. Send them after both apps are running. Preserve genuine agent output in the footage; rehearse or rerecord if an agent takes an unsuitable approach.

## Arrange the recording

Use two compositions, with readable text in each:

- Startup: two labeled terminal sessions beside the browser. Keep the checkout names and `parcela up` output readable.
- Review: give the browser most of the frame. Enlarge the Parcela side tab and the environment list during the switch.

Capture a 16:9 area at 2560×1440 if the recorder and display support it. Export a 1920×1080 delivery version and check it at the actual landing-page player size. These are starting settings, not a requirement to buy a display.

Use a terminal font around 20–24 points as a starting point. Adjust it using a test export. Avoid a permanent four-pane layout where every label becomes tiny.

Use a clean browser profile, close unrelated tabs, and enable Do Not Disturb before the take. Keep app windows in fixed positions. Record without microphone or system audio for the first take; add narration later.

## Capture these shots

Times describe the edited film, not the time the agents need to finish their work.

| Time | On screen | What the viewer should understand |
| --- | --- | --- |
| 0–7s | The shop project and two clearly labeled worktrees, with Claude and Codex sessions | Both agents are working on the same app in different checkouts. |
| 7–18s | Run `parcela up --detach` in each checkout. Hold on the two real HTTPS URLs. | Each worktree starts its own running environment. |
| 18–28s | Open both URLs and briefly show the matching, unchanged shop pages. | Both environments work before any UI edits. |
| 28–45s | Send the tasks. Show a short stretch of real agent work, then the banner appearing in one preview and the list appearing in the other. | Each agent changes its own app. Use a small "Time passes" label at a substantial cut. |
| 45–63s | Enlarge one browser. Open the Parcela side tab. Show the current environment, select the other ready service, and hold on the changed URL and page. | Parcela provides a way to move between running worktrees. |
| 63–74s | Use the side tab to switch back. Hold on the blue banner and unchanged grid, then briefly compare with the list version. | Both apps remain available, and their changes stay separate. |
| 74–80s | Finish on the app with the side tab visible and a short `parcela.run` caption. | The viewer knows the next destination. |

Record the whole workflow, including the waits, as source footage. Edit it into the shot list afterward. Capture five seconds before and after each important action to give yourself room for cuts.

For the side-tab sequence, make one continuous take from opening the panel through loading the other app. Keep the address bar visible. Disable or adjust automatic zoom when it would crop out the environment names or destination URL.

## Add narration after the rough edit

First assemble a silent version that tells the story. Then adjust the narration to the footage you actually captured.

Audition narration before choosing the engine. Kokoro is a possible local option, not an approved final voice.

1. Generate a 15–20 second excerpt containing "Parcela", "Claude", "Codex", "worktree", and a complete explanatory sentence.
2. Compare two or three supplied voices on the same text. Listen for pronunciation, cadence, and whether the delivery feels appropriate for the product.
3. Import one WAV into Cap's Audio track. Test trimming, alignment, volume, and export with the actual recording.
4. If the audition succeeds, generate one short WAV per narration paragraph using the same voice and settings.
5. Align the clips in Cap. Leave small gaps where an action needs time to register, and keep the spoken audio at its natural speed.
6. Add or generate captions from the final audio, then correct names and timing. Confirm how the installed version treats imported narration during transcription; manual captions remain an option.
7. Export and review the result at landing-page size.

Keep the voice decision open if the test sounds synthetic or needs too many pronunciation repairs. No narration has been generated or auditioned for this draft.

Use this direction:

> Calm developer explaining a tool to another developer. Conversational, clear, and unhurried. Leave space around commands and environment names. Avoid an advertising announcer delivery.

Test a short line containing "Parcela", "Claude", "Codex", and "worktree" first. Confirm how you want Parcela pronounced, then use phonetic spelling or a pronunciation setting if necessary. Keep code spelled correctly in captions even if the narration input uses phonetics.

If Kokoro passes the audition, use one of its supplied voices and keep its license information with the production notes. Voice cloning is outside this first recording plan.

Suggested narration, to revise after the first take:

> Here's one shop, checked out in two worktrees. Claude is working in one, and Codex in the other.
>
> First, I start both with Parcela. Each worktree gets its own processes, ports, and HTTPS address.
>
> Both start with the same page. Now I ask Claude to add a sale banner, while Codex changes the catalog layout.
>
> Their apps keep running as they work. The changes appear in separate previews.
>
> Parcela's optional side tab keeps those environments within reach. I can see which one I'm in and switch straight to another running app.
>
> Back in Claude's worktree, the banner is still here and the grid is unchanged. Codex's list lives in its own checkout.
>
> Run your app in every worktree. Get started at parcela.run.

Give the film breathing room where the script stops. Do not stretch every sentence to fill its assigned shot. Omit background music for the first version.

## Finish the edit

1. Cut idle time and mark substantial time skips. Keep the real ordering of commands and results.
2. Add restrained zooms for the terminal URLs and the side tab. Keep enough context to recognize the app.
3. Match the narration to the exact action on screen. Add a short hold after each navigation.
4. Generate captions from the final audio, then correct product names, commands, and worktree names.
5. Watch the exported file muted at landing-page size. The startup, distinct edits, and environment switching must remain understandable.
6. Listen to the final narration and check pronunciation. This draft has not generated or auditioned audio.
7. Keep a high-quality master. Export a web-friendly H.264 MP4, a poster image, and captions for delivery. Supply WebVTT for the site's HTML video track; convert an SRT export if needed.
8. Keep the editable recording project, raw takes, narration text, and separate audio clips together outside source control. Decide the final asset location during website integration.

Use click-to-play video on the landing page, with a clear poster and captions. A later 15–20 second silent excerpt can reuse the strongest shots.

## Make a rough take first

Record only startup and the continuous side-tab switch before polishing the full film. This short take answers the most useful questions: can visitors read the URLs, understand which worktree is open, and see the switch happen?

