* Remove the duplicated helpers on AppViewModel. status, menu_items, downloaded_files and exported_files repeat what AppSnapshot already computes. Nothing outside the crate seems to call them, so delete them, or limit them to tests if the crate's own tests depend on them.
* Add a comment on perform in app_runtime.rs. Each command runs as its own task, so async commands can interleave. Anyone adding one has to handle the races, like LibraryViewModel's generation counter and sign_out suspending jobs first.
* Maybe adjust the AppError API. That's only needed if the root crate drops its own copy of AppError. Whatever extra the UI needs from errors would move here, and I haven't checked yet what that is.

Not needed:
* The threading model, Changes, the command/snapshot design and the test setup can stay as they are.
* Snapshot capture can stay a full recapture until profiling shows a problem.

Outside this crate (the bigger change):
* Fold LiveApp and MockApp in src/model/ down to something closer to reading AppSnapshot directly.
* Run a mock or preview mode as start_app with the fakes from loudible-test-support, instead of MockApp's own reimplementation.
