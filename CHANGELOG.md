# Changelog

## [0.1.0-alpha.7](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.6...v0.1.0-alpha.7) (2026-11-02)


### Features

* add an optional network to execution environments ([#191](https://github.com/koreograf/koreograf/issues/191)) ([9129e96](https://github.com/koreograf/koreograf/commit/9129e9602f6b2a01759ae42939abc68f286a36a2))
* add launch with options for templates without prompts ([#189](https://github.com/koreograf/koreograf/issues/189)) ([ba99433](https://github.com/koreograf/koreograf/commit/ba99433dac724820cd41c855eaed1fb45c4c196d))
* edit a project name, source, and revision ([#195](https://github.com/koreograf/koreograf/issues/195)) ([6fc9241](https://github.com/koreograf/koreograf/commit/6fc92417621a6b8c3d97b85bf9537485353b1daf))
* explain NetBox errors and add a test connection ([#186](https://github.com/koreograf/koreograf/issues/186)) ([b1efcde](https://github.com/koreograf/koreograf/commit/b1efcdefe3cfc0dbcbcff76d00ae107a4c907615))
* include the git commit in the reported version ([#166](https://github.com/koreograf/koreograf/issues/166)) ([237b613](https://github.com/koreograf/koreograf/commit/237b613f91a36a529ac4a2a634825344f05b104d))


### Bug Fixes

* add skip link, sign-in autofocus and restore language focus ([#180](https://github.com/koreograf/koreograf/issues/180)) ([2ce068d](https://github.com/koreograf/koreograf/commit/2ce068d57631daad190eda699b60eda064eb3b97))
* correct workflow editor labels and keyboard support ([#190](https://github.com/koreograf/koreograf/issues/190)) ([7650bf8](https://github.com/koreograf/koreograf/commit/7650bf8334e8e4adb5bf0213ac64ed29c5db7a15))
* delete auto-created netbox credentials with their inventory ([d271def](https://github.com/koreograf/koreograf/commit/d271defdc6736ab30bba6c53f04a07406882ccb4))
* drop stale renders, preselect a useful team, align NetBox checks ([#185](https://github.com/koreograf/koreograf/issues/185)) ([563c27e](https://github.com/koreograf/koreograf/commit/563c27e5c682441246e24b065c9cf468360b53c3))
* explain how webhook calls can start jobs on a default install ([02bbf89](https://github.com/koreograf/koreograf/commit/02bbf893b8537f22e90d3176e27e41869b5d3b53))
* guard auto credential cleanup against json references ([51b3e6e](https://github.com/koreograf/koreograf/commit/51b3e6ede6176e9171acc24771bd1c10b337f85c))
* let pages shrink on narrow windows without horizontal scroll ([#183](https://github.com/koreograf/koreograf/issues/183)) ([534baa2](https://github.com/koreograf/koreograf/commit/534baa29ebf3d30bdf571931e8bafc30a116ed56))
* localize the job log gap and keep the duration current ([#188](https://github.com/koreograf/koreograf/issues/188)) ([dc67aea](https://github.com/koreograf/koreograf/commit/dc67aea0f23aa2d83c4f7d5b2d0cb452b0d112d9))
* make overlay dialogs keyboard-modal with focus trap ([#181](https://github.com/koreograf/koreograf/issues/181)) ([bc382f0](https://github.com/koreograf/koreograf/commit/bc382f096ca72a88531a02e85cf185225f23ae13))
* name deleted objects and separate danger buttons ([#150](https://github.com/koreograf/koreograf/issues/150)) ([#187](https://github.com/koreograf/koreograf/issues/187)) ([0006bbd](https://github.com/koreograf/koreograf/commit/0006bbd778044a871396d6854b976c6e7db489ff))
* refuse to delete an inventory that templates still use ([81c6256](https://github.com/koreograf/koreograf/commit/81c6256134484f9061264599fc49a676bd6abe75))
* reject launching an ansible template without an inventory ([e715520](https://github.com/koreograf/koreograf/commit/e715520103cf674b937c9c5dcebbae9ceb7be67f))
* reject unknown JSON fields in request bodies ([#156](https://github.com/koreograf/koreograf/issues/156)) ([33feacd](https://github.com/koreograf/koreograf/commit/33feacd57adf81c52bf80d78f2319cac0b8012d3))
* reject workflow hook deliveries when local runs are admin-only ([d9da27b](https://github.com/koreograf/koreograf/commit/d9da27bbfea3b79cbcb109460e0d97b22acd1ae5))
* report LDAP outages as 503 and cap NetBox connect timeout ([ba3b6c0](https://github.com/koreograf/koreograf/commit/ba3b6c01a7f2c384e1d2303ce63cb121004cc991))
* save the playbook path shown in the template form ([dfc3535](https://github.com/koreograf/koreograf/commit/dfc35357999c8ebd26b134b8f49029def248998e)), closes [#147](https://github.com/koreograf/koreograf/issues/147)
* scope reference scan to team and cover guard with a test ([ef58d2a](https://github.com/koreograf/koreograf/commit/ef58d2ace2c687ba05120a86060f3b69a9a3c377))
* shift schedule runs out of daylight saving gaps ([ca5e53d](https://github.com/koreograf/koreograf/commit/ca5e53d3e29f02bc6cce48bd32d77ff2561f7a03))
* show errors next to their field and stop banner stacking ([#182](https://github.com/koreograf/koreograf/issues/182)) ([90f9af5](https://github.com/koreograf/koreograf/commit/90f9af50dde12ca8d38b3e3b92887a88c567a072))
* tighten validation for inventories, settings, names, secret key ([#184](https://github.com/koreograf/koreograf/issues/184)) ([03af82e](https://github.com/koreograf/koreograf/commit/03af82eb3130cd818e92d88278d22062a0ff8544))

## [0.1.0-alpha.6](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.5...v0.1.0-alpha.6) (2026-10-28)


### Features

* add an end-to-end test harness ([#115](https://github.com/koreograf/koreograf/issues/115)) ([9a377a8](https://github.com/koreograf/koreograf/commit/9a377a82142dcc0514701c9f6916b1cd684a1a31)), closes [#27](https://github.com/koreograf/koreograf/issues/27)
* add standard response headers ([#106](https://github.com/koreograf/koreograf/issues/106)) ([59a7e22](https://github.com/koreograf/koreograf/commit/59a7e22d4516b2f061556fe8c34ed2a943a1d103))
* ask for launch options before a job starts ([#111](https://github.com/koreograf/koreograf/issues/111)) ([e7a2879](https://github.com/koreograf/koreograf/commit/e7a2879ff769e7b97ce8fb0ef6c1211e9291bfec)), closes [#21](https://github.com/koreograf/koreograf/issues/21)
* expose the version via --version, image labels and the API ([#128](https://github.com/koreograf/koreograf/issues/128)) ([57c4492](https://github.com/koreograf/koreograf/commit/57c44928e88c02c873d6975d16ff345956fc5fbb)), closes [#94](https://github.com/koreograf/koreograf/issues/94)
* polish forms, timestamps and dangerous actions ([#118](https://github.com/koreograf/koreograf/issues/118)) ([143b482](https://github.com/koreograf/koreograf/commit/143b4824d29b0caf3a01c182485fe470b3825c09)), closes [#33](https://github.com/koreograf/koreograf/issues/33)
* relaunch a canceled job from its parameters ([#116](https://github.com/koreograf/koreograf/issues/116)) ([6d8bd73](https://github.com/koreograf/koreograf/commit/6d8bd738f35021dc0b5dd7829e6d99f28a72c347)), closes [#29](https://github.com/koreograf/koreograf/issues/29)
* set Secure on the session cookie behind TLS ([#107](https://github.com/koreograf/koreograf/issues/107)) ([c571e9d](https://github.com/koreograf/koreograf/commit/c571e9d5ffddb124765d1375576f74d97a9eeb5a))
* warn when an Ansible tag filter matches no tasks ([#117](https://github.com/koreograf/koreograf/issues/117)) ([d071ac2](https://github.com/koreograf/koreograf/commit/d071ac26efc2fe8475ad4615e642bf90da54f431)), closes [#31](https://github.com/koreograf/koreograf/issues/31)
* warn when certificate checks are off and add a CA file ([#105](https://github.com/koreograf/koreograf/issues/105)) ([edd21c0](https://github.com/koreograf/koreograf/commit/edd21c0286410c0f4dd43eaba45e541d38b7f288))


### Bug Fixes

* check cleanup file names once in the demo role ([#100](https://github.com/koreograf/koreograf/issues/100)) ([c7c04a8](https://github.com/koreograf/koreograf/commit/c7c04a8c10c01c4b382537beb40bcac3cde2830a))
* check downloads, drop root, and bind compose to localhost ([#101](https://github.com/koreograf/koreograf/issues/101)) ([790aa48](https://github.com/koreograf/koreograf/commit/790aa48e85dc7d525a2cb9bf4a4eb571c770e17e))
* check team membership before starting a scheduled run ([#127](https://github.com/koreograf/koreograf/issues/127)) ([8000616](https://github.com/koreograf/koreograf/commit/80006168d0fb82f75d572fc2d313e070ceb63bcf)), closes [#74](https://github.com/koreograf/koreograf/issues/74)
* count jobs that finish before execution ([#120](https://github.com/koreograf/koreograf/issues/120)) ([6fffff3](https://github.com/koreograf/koreograf/commit/6fffff3bfb1aa5fba8d62d997938470ba640611c)), closes [#47](https://github.com/koreograf/koreograf/issues/47)
* distinguish JSON body errors ([#123](https://github.com/koreograf/koreograf/issues/123)) ([e1d2501](https://github.com/koreograf/koreograf/commit/e1d2501291583f49da10aca41171bb80ab3782cb)), closes [#52](https://github.com/koreograf/koreograf/issues/52)
* keep a missed schedule run when starting it fails ([#125](https://github.com/koreograf/koreograf/issues/125)) ([5551a92](https://github.com/koreograf/koreograf/commit/5551a92e668ced2f5ff537e28d116c98f383c270)), closes [#71](https://github.com/koreograf/koreograf/issues/71)
* keep opentofu state for the team and template ([#109](https://github.com/koreograf/koreograf/issues/109)) ([17d4661](https://github.com/koreograf/koreograf/commit/17d466170179de186e34b2f573fe82c207e59202)), closes [#18](https://github.com/koreograf/koreograf/issues/18)
* keep the start and end of job output ([#110](https://github.com/koreograf/koreograf/issues/110)) ([95cab85](https://github.com/koreograf/koreograf/commit/95cab85e3de2fb0caad2a7ea223adb81c57a597e)), closes [#19](https://github.com/koreograf/koreograf/issues/19)
* launch scheduled runs as the user who last edited the schedule ([#126](https://github.com/koreograf/koreograf/issues/126)) ([d73ae76](https://github.com/koreograf/koreograf/commit/d73ae768c519533e7890065e2f8d632bc09f2752)), closes [#73](https://github.com/koreograf/koreograf/issues/73)
* limit description length to 1000 characters ([#108](https://github.com/koreograf/koreograf/issues/108)) ([7b5adcf](https://github.com/koreograf/koreograf/commit/7b5adcf340dbbdc875b926f6c58385706079b506))
* log the reason when an image pull fails ([#133](https://github.com/koreograf/koreograf/issues/133)) ([4cf8792](https://github.com/koreograf/koreograf/commit/4cf87921cfcc11b00bb48a6ad7728653ccfce561)), closes [#99](https://github.com/koreograf/koreograf/issues/99)
* mark the real failure in the job log ([#113](https://github.com/koreograf/koreograf/issues/113)) ([4f69833](https://github.com/koreograf/koreograf/commit/4f69833921c715b7d3101596f80aadb0a364170b)), closes [#23](https://github.com/koreograf/koreograf/issues/23)
* mount execution environments from the daemon data path ([#129](https://github.com/koreograf/koreograf/issues/129)) ([106bcd1](https://github.com/koreograf/koreograf/commit/106bcd150713f76a80ba420ac4a202d17183eca9)), closes [#95](https://github.com/koreograf/koreograf/issues/95)
* reap orphaned job processes with tini ([#119](https://github.com/koreograf/koreograf/issues/119)) ([99df0e9](https://github.com/koreograf/koreograf/commit/99df0e9787375c91223315062a0f53c6ea2b6041)), closes [#46](https://github.com/koreograf/koreograf/issues/46)
* run execution environments as the job directory owner ([#130](https://github.com/koreograf/koreograf/issues/130)) ([6d29c98](https://github.com/koreograf/koreograf/commit/6d29c9868b7092e4e2f2c0f10a620276cbe2a383)), closes [#96](https://github.com/koreograf/koreograf/issues/96)
* say when the username or password is wrong ([#114](https://github.com/koreograf/koreograf/issues/114)) ([c2ef369](https://github.com/koreograf/koreograf/commit/c2ef369debbba70ddfc790b119749389df1739b8)), closes [#26](https://github.com/koreograf/koreograf/issues/26)
* show an error when the playbook path is empty ([#112](https://github.com/koreograf/koreograf/issues/112)) ([ef57492](https://github.com/koreograf/koreograf/commit/ef57492812440b26cede97f30811d4e385f15656)), closes [#22](https://github.com/koreograf/koreograf/issues/22)
* use the directory name for LDAP accounts ([#121](https://github.com/koreograf/koreograf/issues/121)) ([9ac6b0f](https://github.com/koreograf/koreograf/commit/9ac6b0fc8e6fddba57866611ccc2057db6247e3c)), closes [#50](https://github.com/koreograf/koreograf/issues/50)
* validate netbox addresses as IpAddr before writing ansible_host ([#124](https://github.com/koreograf/koreograf/issues/124)) ([d351ee1](https://github.com/koreograf/koreograf/commit/d351ee1e3a7ba7fa8f53a5825b61fe7340c06789)), closes [#69](https://github.com/koreograf/koreograf/issues/69)
* warn when auto mode ignores an environment ([#131](https://github.com/koreograf/koreograf/issues/131)) ([18802f6](https://github.com/koreograf/koreograf/commit/18802f6769118f99eb7a73d3c09b636574e5b41b)), closes [#97](https://github.com/koreograf/koreograf/issues/97)

## [0.1.0-alpha.5](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.4...v0.1.0-alpha.5) (2026-10-26)


### Bug Fixes

* allow editing a schedule with an old start date ([#75](https://github.com/koreograf/koreograf/issues/75)) ([05be71d](https://github.com/koreograf/koreograf/commit/05be71d11c392c8d9df3a0d2d4c95ef234cf7d4f)), closes [#72](https://github.com/koreograf/koreograf/issues/72)
* **deps:** pin debian docker tag to 7ff6ba3 ([#77](https://github.com/koreograf/koreograf/issues/77)) ([5e8acfd](https://github.com/koreograf/koreograf/commit/5e8acfdc04e669d632b0d44393a54eaf5d38a3ea))
* **deps:** update cargo non-major ([#87](https://github.com/koreograf/koreograf/issues/87)) ([4e0bdd5](https://github.com/koreograf/koreograf/commit/4e0bdd575ae64a10c204a39aafde6551bafef56d))
* **deps:** update dependency opentofu to v1.13.2 ([#83](https://github.com/koreograf/koreograf/issues/83)) ([1c967dd](https://github.com/koreograf/koreograf/commit/1c967dd4a8545c176a9fc8e7b21bdc38b425bb0d))
* pull the published image in compose ([#91](https://github.com/koreograf/koreograf/issues/91)) ([65b76fd](https://github.com/koreograf/koreograf/commit/65b76fde2bae46e4104339090f95cb4d7f7d770e))
* use debian codenames in the demo role metadata ([#89](https://github.com/koreograf/koreograf/issues/89)) ([afd6ac1](https://github.com/koreograf/koreograf/commit/afd6ac16fdc7fccfe5aaf92323bc608281f0d834))

## [0.1.0-alpha.4](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.3...v0.1.0-alpha.4) (2026-10-23)


### Features

* schedule templates and workflows at a local time ([#66](https://github.com/koreograf/koreograf/issues/66)) ([7e9c955](https://github.com/koreograf/koreograf/commit/7e9c955943677452c883dc82b052e07b57f7ecac))


### Bug Fixes

* make netbox inventory group names ansible-safe ([#64](https://github.com/koreograf/koreograf/issues/64)) ([ab0a806](https://github.com/koreograf/koreograf/commit/ab0a8064177ffd38b82f37bca84a764e1a8096ad)), closes [#25](https://github.com/koreograf/koreograf/issues/25)
* set ansible_host for ipv6-only netbox devices ([#65](https://github.com/koreograf/koreograf/issues/65)) ([3ad2528](https://github.com/koreograf/koreograf/commit/3ad2528b2afcf9c0825bca4258b33fedc646c009)), closes [#48](https://github.com/koreograf/koreograf/issues/48)

## [0.1.0-alpha.3](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.2...v0.1.0-alpha.3) (2026-10-23)


### ⚠ BREAKING CHANGES

* Local jobs on the controller now default to administrators. An install that previously let operators, webhooks, or workflow nodes start those jobs sets `KOREOGRAF_LOCAL_RUNNER_FOR=all`. Every path project runs only when its directory is under an allowed root, including one an administrator saved. The examples directory is always allowed, so the seeded demo keeps running. Other directories go in `KOREOGRAF_PROJECT_ROOTS`. An empty LDAP service bind password fails sign-in with `ldap_is_not_fully_configured`.

### Bug Fixes

* close the five draft security advisories ([#53](https://github.com/koreograf/koreograf/issues/53)) ([6b29bdc](https://github.com/koreograf/koreograf/commit/6b29bdc6093af4371bdf4598790271890ea23e76))

## [0.1.0-alpha.2](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.1...v0.1.0-alpha.2) (2026-10-22)


### ⚠ BREAKING CHANGES

* error responses use a message key in error, plus optional vars, instead of a German sentence.

### Features

* let a template skip the git update on launch ([b22c75a](https://github.com/koreograf/koreograf/commit/b22c75ad7286128e4ac9529a4b1e1f0162c7438f))
* move visible copy into message catalogs ([3b2a6d0](https://github.com/koreograf/koreograf/commit/3b2a6d0f7fbbf78746c4c16a3a045c255e1a91a3))


### Bug Fixes

* forward log, database, and proxy settings in Compose ([#41](https://github.com/koreograf/koreograf/issues/41)) ([2995677](https://github.com/koreograf/koreograf/commit/29956778a99e06ecbea2e0c9e654ef68cadd6f47))
* keep demo role defaults when alert vars are missing ([#45](https://github.com/koreograf/koreograf/issues/45)) ([3c0fc3d](https://github.com/koreograf/koreograf/commit/3c0fc3df8876d176cf23159aac096c624efface8))
* return a catalog error when the JSON body is missing ([5e54e45](https://github.com/koreograf/koreograf/commit/5e54e45b09d03323916320fd2a2847721fb365c4)), closes [#30](https://github.com/koreograf/koreograf/issues/30)
* run local jobs from a relative data directory ([d229084](https://github.com/koreograf/koreograf/commit/d229084c0d2000fae085995efd78102d36918404)), closes [#36](https://github.com/koreograf/koreograf/issues/36) [#37](https://github.com/koreograf/koreograf/issues/37) [#17](https://github.com/koreograf/koreograf/issues/17)
* search LDAP groups with the service account ([#40](https://github.com/koreograf/koreograf/issues/40)) ([0d61ea1](https://github.com/koreograf/koreograf/commit/0d61ea19da5d72189980ef48c931028c563d4815)), closes [#20](https://github.com/koreograf/koreograf/issues/20) [#28](https://github.com/koreograf/koreograf/issues/28) [#51](https://github.com/koreograf/koreograf/issues/51)
* stop a canceled job by its process group ([#39](https://github.com/koreograf/koreograf/issues/39)) ([9197054](https://github.com/koreograf/koreograf/commit/9197054e385462454e66e9a6f068b42412cddd1f))

## 0.1.0-alpha.1

First public alpha. One Rust binary serves the API, the German UI, and the job runner. English is available from the language menu.

This release can run Ansible, OpenTofu, and shell jobs for teams in organizations. It stores credentials, inventories (including a project file and NetBox), and projects. Execution environments are global, per organization, or private to one team. The Galaxy, role, and OpenTofu cache stays under `data/cache/<team>/`. Workflows are directed graphs. Alertmanager and Grafana webhooks can start a job or a workflow. An administrator can set the directory, the HTTP proxy, the runner, and the job timeout, and can download or restore a backup of the SQLite file and `secret.key`. `GET /metrics` exposes Prometheus text.

Limits of this alpha: one SQLite file and one writer, so the Helm chart stays at one replica. There are no surveys, no schedules, no Receptor, and no high-availability setup. A `postgres://` URL stops startup. The container image is `linux/amd64`. The prototype expects a trusted network. The Docker socket is off unless you turn it on, and then it is root on the host.
