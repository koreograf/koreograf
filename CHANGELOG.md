# Changelog

## [0.1.0-alpha.11](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.10...v0.1.0-alpha.11) (2026-11-11)


### Bug Fixes

* pass the project scm credential to the galaxy step of jobs ([#403](https://github.com/koreograf/koreograf/issues/403)) ([f9eb428](https://github.com/koreograf/koreograf/commit/f9eb4288af0ee4031afdb9df3940da7990569be7))
* stop blocking tokio workers in the postgresql backend ([#405](https://github.com/koreograf/koreograf/issues/405)) ([1588a56](https://github.com/koreograf/koreograf/commit/1588a56ab6cd3bf33bd99e63a73a9838cdecd553))

## [0.1.0-alpha.10](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.9...v0.1.0-alpha.10) (2026-11-10)


### Features

* add a postgresql backend foundation ([#301](https://github.com/koreograf/koreograf/issues/301)) ([12eaa16](https://github.com/koreograf/koreograf/commit/12eaa16e6b49887c93c44afe8ae518c607107718))
* add optional read-only llm assistant chat ([#309](https://github.com/koreograf/koreograf/issues/309)) ([#369](https://github.com/koreograf/koreograf/issues/369)) ([053ee6f](https://github.com/koreograf/koreograf/commit/053ee6f879eff05e4241d8662e83de53583bac0e))
* add optional token auth for metrics ([#347](https://github.com/koreograf/koreograf/issues/347)) ([2b1b496](https://github.com/koreograf/koreograf/commit/2b1b4960d44d5dad1c00f61a3bcd5a08a622de51))
* add the postgresql migration wizard ([#300](https://github.com/koreograf/koreograf/issues/300)) ([#372](https://github.com/koreograf/koreograf/issues/372)) ([6977645](https://github.com/koreograf/koreograf/commit/697764592929c9026e6e4fe75883abe2bfe03f92))
* add workflow nodes from a context menu ([#365](https://github.com/koreograf/koreograf/issues/365)) ([2705ea2](https://github.com/koreograf/koreograf/commit/2705ea2899f47d71bce945facc7b4ba52f266830))
* back up and migrate databases on the postgres backend ([#313](https://github.com/koreograf/koreograf/issues/313)) ([893ea0a](https://github.com/koreograf/koreograf/commit/893ea0a09790fc1f3ebaafe08f6f478fb396127b))
* confirm opentofu apply launches ([#356](https://github.com/koreograf/koreograf/issues/356)) ([2aad266](https://github.com/koreograf/koreograf/commit/2aad2664c0595bc038d6698361fa6f413dc50fd4))
* connect workflow edges from a node menu ([#364](https://github.com/koreograf/koreograf/issues/364)) ([0c18c5b](https://github.com/koreograf/koreograf/commit/0c18c5b3c48210f2ea51ff8351d913c84a03b0de))
* create schedules from the schedules page and a template ([#325](https://github.com/koreograf/koreograf/issues/325)) ([#359](https://github.com/koreograf/koreograf/issues/359)) ([3e5a22c](https://github.com/koreograf/koreograf/commit/3e5a22cf40dbf6f83c6697f23eb0392f5df5386e))
* finish the llm assistant settings, tools, and writes ([#309](https://github.com/koreograf/koreograf/issues/309)) ([#370](https://github.com/koreograf/koreograf/issues/370)) ([1107136](https://github.com/koreograf/koreograf/commit/1107136274535078d1c5f6e61ab61bed4fcdcf8f))
* give job containers the controller CA bundle ([5cf6232](https://github.com/koreograf/koreograf/commit/5cf623293c417bdfc258125da658501bf618fe5d))
* port credentials and projects to the postgres backend ([#302](https://github.com/koreograf/koreograf/issues/302)) ([28b9762](https://github.com/koreograf/koreograf/commit/28b9762502efb243a49b76c7c443dcd07aff992b))
* port jobs and their output to the postgres backend ([#312](https://github.com/koreograf/koreograf/issues/312)) ([1c12f8a](https://github.com/koreograf/koreograf/commit/1c12f8adae2b9596ce91f81616791c088b1e6999))
* port templates and schedules to the postgres backend ([#303](https://github.com/koreograf/koreograf/issues/303)) ([59de955](https://github.com/koreograf/koreograf/commit/59de955a6cdbcb4324a8ee35ad74f893b8ecc52a))
* port users and the overview to the postgres backend ([#311](https://github.com/koreograf/koreograf/issues/311)) ([032059e](https://github.com/koreograf/koreograf/commit/032059ecca80cf43ab4dd4d54eb353fb342de743))
* port workflows and hook delivery to the postgres backend ([#310](https://github.com/koreograf/koreograf/issues/310)) ([b086887](https://github.com/koreograf/koreograf/commit/b08688703186b595133171d6a971ea989e738127))
* publish deb and rpm packages ([#299](https://github.com/koreograf/koreograf/issues/299)) ([e447024](https://github.com/koreograf/koreograf/commit/e4470244d17366a789bfcfd4f14b05b6147245f2))
* put settings into tab groups ([#367](https://github.com/koreograf/koreograf/issues/367)) ([f011f87](https://github.com/koreograf/koreograf/commit/f011f87ef62b77b6df27eca48ef2db1fff2cdda0))
* put workflow schedules and danger zone in tabs ([#363](https://github.com/koreograf/koreograf/issues/363)) ([5c1fc7b](https://github.com/koreograf/koreograf/commit/5c1fc7bf735c1f842c379dc131e8d02d62002de2))
* throttle repeated failed sign-ins ([#349](https://github.com/koreograf/koreograf/issues/349)) ([4aa106d](https://github.com/koreograf/koreograf/commit/4aa106d9e2fc32231098f268fee3cb61e99da3e3))


### Bug Fixes

* **deps:** update debian:bookworm-slim docker digest to 734a72b ([#295](https://github.com/koreograf/koreograf/issues/295)) ([8570279](https://github.com/koreograf/koreograf/commit/8570279e48d07221129b785d8e118f83e5cdf4fe))
* **deps:** update rust:1.99.1-alpine docker digest to 44819b2 ([#296](https://github.com/koreograf/koreograf/issues/296)) ([e6ba2ae](https://github.com/koreograf/koreograf/commit/e6ba2ae44d7c7056e65dc421214b468a56a817a4))
* **deps:** update the docker cli to 29.9.3 ([#298](https://github.com/koreograf/koreograf/issues/298)) ([d4f50f6](https://github.com/koreograf/koreograf/commit/d4f50f60d8851ff2f266d9515f2fdd43e4135190))
* gate launch extra vars on prompt or options ([#348](https://github.com/koreograf/koreograf/issues/348)) ([3345c4b](https://github.com/koreograf/koreograf/commit/3345c4bf7caa647a57cd024b84dd297cb8d557a1))
* guide empty states and contain long job logs ([#362](https://github.com/koreograf/koreograf/issues/362)) ([d440de7](https://github.com/koreograf/koreograf/commit/d440de7364177202b8f1763d4a4a83a9242ce3e3))
* harden names and hide disabled sign-in state ([#338](https://github.com/koreograf/koreograf/issues/338)) ([#353](https://github.com/koreograf/koreograf/issues/353)) ([6b239c1](https://github.com/koreograf/koreograf/commit/6b239c17a716947368814a0f2788be2212c29c77))
* ignore the read-only fields on settings updates ([#342](https://github.com/koreograf/koreograf/issues/342)) ([9f8a2e2](https://github.com/koreograf/koreograf/commit/9f8a2e2c399d02b211f6424830ac2fd5ea6a8ecf))
* keep dirty guard on inventory source edits ([#323](https://github.com/koreograf/koreograf/issues/323)) ([#357](https://github.com/koreograf/koreograf/issues/357)) ([9416edb](https://github.com/koreograf/koreograf/commit/9416edb3dcb0ff54e0dcf1f7c0efd96dc26b4b55))
* load each page once with a loading skeleton ([#324](https://github.com/koreograf/koreograf/issues/324)) ([#358](https://github.com/koreograf/koreograf/issues/358)) ([7da0ea6](https://github.com/koreograf/koreograf/commit/7da0ea62ed86c3fc92f1af73e3ee1abe7188c1b6))
* navigate on first menu click with overlay open ([#350](https://github.com/koreograf/koreograf/issues/350)) ([5c63b67](https://github.com/koreograf/koreograf/commit/5c63b67049e4a34cff3552c4f5692a6cef04153e))
* polish ui details from the audit ([#329](https://github.com/koreograf/koreograf/issues/329)) ([#360](https://github.com/koreograf/koreograf/issues/360)) ([dbc3183](https://github.com/koreograf/koreograf/commit/dbc3183f9d8e7704c439051774dc04ebf3961741))
* prevent a double job from the launch dialog ([#321](https://github.com/koreograf/koreograf/issues/321)) ([#354](https://github.com/koreograf/koreograf/issues/354)) ([df43835](https://github.com/koreograf/koreograf/commit/df438358a560d6573c3536a81579cfac14b2a99f))
* raise muted text contrast to wcag aa ([#328](https://github.com/koreograf/koreograf/issues/328)) ([#355](https://github.com/koreograf/koreograf/issues/355)) ([561428b](https://github.com/koreograf/koreograf/commit/561428b078f0a53ac0b4e4c3066363f079a8ef44))
* reject cross-origin api writes ([#343](https://github.com/koreograf/koreograf/issues/343)) ([1108f08](https://github.com/koreograf/koreograf/commit/1108f081646353b103944547ff38d553163d78ba))
* reject far-future schedule starts ([#361](https://github.com/koreograf/koreograf/issues/361)) ([#368](https://github.com/koreograf/koreograf/issues/368)) ([43ef822](https://github.com/koreograf/koreograf/commit/43ef82272ec2c7cc25d22cbd29daf091c140fec9))
* restore overlay focus and raise click targets ([#330](https://github.com/koreograf/koreograf/issues/330)) ([#351](https://github.com/koreograf/koreograf/issues/351)) ([6b46894](https://github.com/koreograf/koreograf/commit/6b46894507ffdca54330b03bb4947e8626b75561))
* seed demo and curated environments on postgres ([#332](https://github.com/koreograf/koreograf/issues/332)) ([68c7e8b](https://github.com/koreograf/koreograf/commit/68c7e8b4214ed5de794ac08e9911cd7c6497c379))
* ship a minimal shell for file git in the image ([#318](https://github.com/koreograf/koreograf/issues/318)) ([#345](https://github.com/koreograf/koreograf/issues/345)) ([066eb07](https://github.com/koreograf/koreograf/commit/066eb07dbe4ca1c7c9037253ebe4e4f7a26bb281))
* show an error page when a route fails ([#352](https://github.com/koreograf/koreograf/issues/352)) ([de87579](https://github.com/koreograf/koreograf/commit/de875796df189087da2658ded5fd6ffa890f6fac))
* sign in through ldap on postgresql ([#339](https://github.com/koreograf/koreograf/issues/339)) ([8b8eecf](https://github.com/koreograf/koreograf/commit/8b8eecf6f37070c0dd909258f3e50093146606d8))
* stack tables into cards on narrow windows ([#327](https://github.com/koreograf/koreograf/issues/327)) ([#344](https://github.com/koreograf/koreograf/issues/344)) ([cd96ecc](https://github.com/koreograf/koreograf/commit/cd96ecc12c2fe5bab7e6a59c37d1aff73f771456))
* unify project path and foreign id errors ([#336](https://github.com/koreograf/koreograf/issues/336)) ([#346](https://github.com/koreograf/koreograf/issues/346)) ([77045e2](https://github.com/koreograf/koreograf/commit/77045e2b5331d4dc8c069eb3390343a56bb4b8a9))

## [0.1.0-alpha.9](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.8...v0.1.0-alpha.9) (2026-11-06)


### Features

* add a stdio MCP subcommand ([#289](https://github.com/koreograf/koreograf/issues/289)) ([e9a0013](https://github.com/koreograf/koreograf/commit/e9a0013612a74611d7d0835e0ca31c4acedcfec5))
* add personal api tokens for bearer auth ([#287](https://github.com/koreograf/koreograf/issues/287)) ([c82fa1f](https://github.com/koreograf/koreograf/commit/c82fa1f5f2af2333378978d0fbcd8e1671aa968b))
* serve mcp over http ([#290](https://github.com/koreograf/koreograf/issues/290)) ([7de4181](https://github.com/koreograf/koreograf/commit/7de41818b432dacdaaf9cdda1ffa23f35cab78f5))


### Bug Fixes

* give the job container a passwd entry for its uid ([6dcc15c](https://github.com/koreograf/koreograf/commit/6dcc15c673ac21a1cfffe6467927b1dd9f11d432))

## [0.1.0-alpha.8](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.7...v0.1.0-alpha.8) (2026-11-05)


### ⚠ BREAKING CHANGES

* existing Docker installs must add the socket group with group_add, a saved local runner mode is switched to docker, and templates without an environment use the seeded default per tool.

### Features

* add dashboard tab with job charts ([#273](https://github.com/koreograf/koreograf/issues/273)) ([#275](https://github.com/koreograf/koreograf/issues/275)) ([026ac86](https://github.com/koreograf/koreograf/commit/026ac86d18371e46fbddac2306e48e551fda0490))
* add edit overlays for credentials and admin objects ([#243](https://github.com/koreograf/koreograf/issues/243)) ([680a540](https://github.com/koreograf/koreograf/commit/680a540509ea7d7d0121dca9293871b7cd0590e3))
* add edit overlays for projects and inventories ([#244](https://github.com/koreograf/koreograf/issues/244)) ([48c6684](https://github.com/koreograf/koreograf/commit/48c6684a6f2809863f3f50c13cc246c752ed6ce3))
* add paging and filters to the job list ([#232](https://github.com/koreograf/koreograf/issues/232)) ([a8bf839](https://github.com/koreograf/koreograf/commit/a8bf839ab950cbed14bf5d97fccaee9aa74bebc3))
* add the job output summary and search ([#230](https://github.com/koreograf/koreograf/issues/230)) ([#233](https://github.com/koreograf/koreograf/issues/233)) ([3cac44d](https://github.com/koreograf/koreograf/commit/3cac44d245b92a43bffc42cd73c75bde9f9b7460))
* create workflows in an overlay ([#249](https://github.com/koreograf/koreograf/issues/249)) ([f388fd2](https://github.com/koreograf/koreograf/commit/f388fd204abf93275f9c8019876994d8aa1ad2c1))
* default execution environments per tool ([#224](https://github.com/koreograf/koreograf/issues/224)) ([#225](https://github.com/koreograf/koreograf/issues/225)) ([fbafd60](https://github.com/koreograf/koreograf/commit/fbafd607f0670bf77545ec16da75217e4daee3ef))
* drop variables column from inventory tables ([#266](https://github.com/koreograf/koreograf/issues/266)) ([3b3bd56](https://github.com/koreograf/koreograf/commit/3b3bd56d49c08ed11ef245247cc8b65ffbc847e4))
* edit templates in a tabbed overlay ([#248](https://github.com/koreograf/koreograf/issues/248)) ([5f906de](https://github.com/koreograf/koreograf/commit/5f906de1d2076cbf42324225cb317271494440f3))
* explain token access in the git project credential ([#267](https://github.com/koreograf/koreograf/issues/267)) ([dbd7e7e](https://github.com/koreograf/koreograf/commit/dbd7e7ed1b8b9320fe7aaa1679d408bf7fda6245))
* move the remaining small forms into overlays ([#245](https://github.com/koreograf/koreograf/issues/245)) ([f14847e](https://github.com/koreograf/koreograf/commit/f14847e64c97de03bc81ef017123bed409c422f5))
* offer credential kinds per tool in templates ([#237](https://github.com/koreograf/koreograf/issues/237)) ([#251](https://github.com/koreograf/koreograf/issues/251)) ([42d7113](https://github.com/koreograf/koreograf/commit/42d7113d08f9434b093e0eb448e8ea12f8a0f123))
* preflight the runner tools and recommend installs ([#257](https://github.com/koreograf/koreograf/issues/257)) ([ae83a1f](https://github.com/koreograf/koreograf/commit/ae83a1f02e04bc1550827b38cbb78a1907fb2005))
* regroup the sidebar and rename runtime entry ([#228](https://github.com/koreograf/koreograf/issues/228)) ([97df6d6](https://github.com/koreograf/koreograf/commit/97df6d6a3feccc661116d0a7f996e90b8f57dd62))
* restore and import a non-Docker installation ([#241](https://github.com/koreograf/koreograf/issues/241)) ([#263](https://github.com/koreograf/koreograf/issues/263)) ([c383715](https://github.com/koreograf/koreograf/commit/c383715c69195b915fc7fb82ec71f57ebf83d1c1))
* seed curated default execution environments ([#239](https://github.com/koreograf/koreograf/issues/239)) ([#259](https://github.com/koreograf/koreograf/issues/259)) ([d0debd5](https://github.com/koreograf/koreograf/commit/d0debd5b08c559c92e8b292c0bda6457cbb6f0f9))
* show inventory variables as yaml and json ([#274](https://github.com/koreograf/koreograf/issues/274)) ([04c2586](https://github.com/koreograf/koreograf/commit/04c2586aed005750b93eb311e06f6f332bb3f16e))
* show the running build in the ui ([#279](https://github.com/koreograf/koreograf/issues/279)) ([07f9375](https://github.com/koreograf/koreograf/commit/07f9375e9dc2673c9132dfe3aa1c4ba6db01a568))
* slim and harden the container image ([#242](https://github.com/koreograf/koreograf/issues/242)) ([75ceb88](https://github.com/koreograf/koreograf/commit/75ceb881d7acf69c51c2c22829463c71a3246fc2))
* trust company CAs from a mounted directory ([#277](https://github.com/koreograf/koreograf/issues/277)) ([676a140](https://github.com/koreograf/koreograf/commit/676a140537d24cd22def9049b733f2393c3b9d94))


### Bug Fixes

* apply form control style to every dialog ([#246](https://github.com/koreograf/koreograf/issues/246)) ([#250](https://github.com/koreograf/koreograf/issues/250)) ([042c39e](https://github.com/koreograf/koreograf/commit/042c39e9ff1d68f04ddf34a3d2bdf734797bf0b0))
* confirm and label credential actions ([#256](https://github.com/koreograf/koreograf/issues/256)) ([4888010](https://github.com/koreograf/koreograf/commit/4888010f16506635b4271c3060ee1fe1e808f562))
* grant contents write to the called release workflow ([#280](https://github.com/koreograf/koreograf/issues/280)) ([3e0d546](https://github.com/koreograf/koreograf/commit/3e0d546b0fa634156d9531452e4a0686d721c3f5))
* keep edit overlay tabs still between panels ([#271](https://github.com/koreograf/koreograf/issues/271)) ([349aadc](https://github.com/koreograf/koreograf/commit/349aadcb2d3064d61da6fe8af8621e23aa5bda91))
* show template vault credentials as searchable pills ([#260](https://github.com/koreograf/koreograf/issues/260)) ([400f206](https://github.com/koreograf/koreograf/commit/400f206cce5b16c3270b5bb7a7e501c82de64bf2))

## [0.1.0-alpha.7](https://github.com/koreograf/koreograf/compare/v0.1.0-alpha.6...v0.1.0-alpha.7) (2026-11-02)


### ⚠ BREAKING CHANGES

* page and search inventory hosts ([#163](https://github.com/koreograf/koreograf/issues/163))

### Features

* add an optional network to execution environments ([#191](https://github.com/koreograf/koreograf/issues/191)) ([9129e96](https://github.com/koreograf/koreograf/commit/9129e9602f6b2a01759ae42939abc68f286a36a2))
* add launch with options for templates without prompts ([#189](https://github.com/koreograf/koreograf/issues/189)) ([ba99433](https://github.com/koreograf/koreograf/commit/ba99433dac724820cd41c855eaed1fb45c4c196d))
* add plain NetBox groups, context and preview ([#162](https://github.com/koreograf/koreograf/issues/162)) ([#203](https://github.com/koreograf/koreograf/issues/203)) ([1030055](https://github.com/koreograf/koreograf/commit/1030055ba9dd1068c60117445b993c63a754819d))
* edit a project name, source, and revision ([#195](https://github.com/koreograf/koreograf/issues/195)) ([6fc9241](https://github.com/koreograf/koreograf/commit/6fc92417621a6b8c3d97b85bf9537485353b1daf))
* edit inventory sources and rename inventories ([#198](https://github.com/koreograf/koreograf/issues/198)) ([903cc0c](https://github.com/koreograf/koreograf/commit/903cc0c7dbc11331899dcf0a0684d58ca53ee78d))
* explain NetBox errors and add a test connection ([#186](https://github.com/koreograf/koreograf/issues/186)) ([b1efcde](https://github.com/koreograf/koreograf/commit/b1efcdefe3cfc0dbcbcff76d00ae107a4c907615))
* include the git commit in the reported version ([#166](https://github.com/koreograf/koreograf/issues/166)) ([237b613](https://github.com/koreograf/koreograf/commit/237b613f91a36a529ac4a2a634825344f05b104d))
* page and search inventory hosts ([#163](https://github.com/koreograf/koreograf/issues/163)) ([afa5a99](https://github.com/koreograf/koreograf/commit/afa5a99ac996e747087429603546a7ef71f5295c))
* scope execution environments to a tool ([#222](https://github.com/koreograf/koreograf/issues/222)) ([50d733f](https://github.com/koreograf/koreograf/commit/50d733fc3a989743e054c9ba7a7a7c97b4968676))
* show inventory host and group details ([#201](https://github.com/koreograf/koreograf/issues/201)) ([daadd7d](https://github.com/koreograf/koreograf/commit/daadd7ded03927bf288e68ba6aecf25908c731f1))
* use a source control credential for project sync ([#205](https://github.com/koreograf/koreograf/issues/205)) ([421effa](https://github.com/koreograf/koreograf/commit/421effa1a45b1b6401695f51a35b82295c75befc))
* use NetBox credentials in inventory sources ([#161](https://github.com/koreograf/koreograf/issues/161)) ([#202](https://github.com/koreograf/koreograf/issues/202)) ([3daa187](https://github.com/koreograf/koreograf/commit/3daa187d3e8cc2034585c43ed84f76fc3aa09fb9))


### Bug Fixes

* add skip link, sign-in autofocus and restore language focus ([#180](https://github.com/koreograf/koreograf/issues/180)) ([2ce068d](https://github.com/koreograf/koreograf/commit/2ce068d57631daad190eda699b60eda064eb3b97))
* correct workflow editor labels and keyboard support ([#190](https://github.com/koreograf/koreograf/issues/190)) ([7650bf8](https://github.com/koreograf/koreograf/commit/7650bf8334e8e4adb5bf0213ac64ed29c5db7a15))
* delete auto-created netbox credentials with their inventory ([d271def](https://github.com/koreograf/koreograf/commit/d271defdc6736ab30bba6c53f04a07406882ccb4))
* **deps:** pin dependencies ([#192](https://github.com/koreograf/koreograf/issues/192)) ([63bdb88](https://github.com/koreograf/koreograf/commit/63bdb88e230b4891dd4737672f669ac0dbd74da0))
* **deps:** update cargo non-major ([#200](https://github.com/koreograf/koreograf/issues/200)) ([2dc7a32](https://github.com/koreograf/koreograf/commit/2dc7a329bde88a56b687fd3c94aa704d357b355c))
* **deps:** update debian:bookworm-slim docker digest to 99c0b25 ([#193](https://github.com/koreograf/koreograf/issues/193)) ([0ae9cc1](https://github.com/koreograf/koreograf/commit/0ae9cc192fac02b753529e5ddf9c4b790214af47))
* **deps:** update rust to v1.99.1 ([#217](https://github.com/koreograf/koreograf/issues/217)) ([47eb612](https://github.com/koreograf/koreograf/commit/47eb61241a4c2da29ab814239e47bf7d4dcacec6))
* drop stale renders, preselect a useful team, align NetBox checks ([#185](https://github.com/koreograf/koreograf/issues/185)) ([563c27e](https://github.com/koreograf/koreograf/commit/563c27e5c682441246e24b065c9cf468360b53c3))
* explain how webhook calls can start jobs on a default install ([02bbf89](https://github.com/koreograf/koreograf/commit/02bbf893b8537f22e90d3176e27e41869b5d3b53))
* guard auto credential cleanup against json references ([51b3e6e](https://github.com/koreograf/koreograf/commit/51b3e6ede6176e9171acc24771bd1c10b337f85c))
* keep git checkouts usable with the Docker socket ([#206](https://github.com/koreograf/koreograf/issues/206)) ([20af3b9](https://github.com/koreograf/koreograf/commit/20af3b94b1bc4155ea0783cc421fd7c54d8856f1))
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
* unblock navigation from unchanged forms ([#207](https://github.com/koreograf/koreograf/issues/207)) ([#208](https://github.com/koreograf/koreograf/issues/208)) ([d74444b](https://github.com/koreograf/koreograf/commit/d74444bb1096bd4bf88e056fb59d13d1032757e5))
* unify overlay dialogs and guard unsaved changes ([#165](https://github.com/koreograf/koreograf/issues/165)) ([#204](https://github.com/koreograf/koreograf/issues/204)) ([7f89b40](https://github.com/koreograf/koreograf/commit/7f89b40c9398f296ce5d2e71216bc4e698c630a1))

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
