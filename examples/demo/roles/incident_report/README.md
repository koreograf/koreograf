# incident_report

Records a self-heal decision in the Koreograf job workspace.

`incident_report_path` and `incident_report_cleanup_dir` are the public API and live in `defaults/main.yml`. The file mode is internal and lives in `vars/main.yml`. Both tasks run on the controller (`delegate_to: localhost`), so a remote inventory cannot delete files on the target. Cleanup accepts only plain file names.
