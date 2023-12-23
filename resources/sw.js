navigator.serviceWorker.getRegistrations().then((function(e){for(let o of e)o.unregister()})),importScripts("https://storage.googleapis.com/workbox-cdn/releases/5.0.0-beta.1/workbox-sw.js"),workbox.core.skipWaiting(),workbox.core.clientsClaim(),workbox.routing.registerRoute(/\.(?:js|css|json5)$/,new workbox.strategies.StaleWhileRevalidate({cacheName:"static-resources"})),workbox.routing.registerRoute(/^https:\/\/fonts\.googleapis\.com/,new workbox.strategies.StaleWhileRevalidate({cacheName:"google-fonts-stylesheets"})),workbox.routing.registerRoute(/^https:\/\/fonts\.gstatic\.com/,new workbox.strategies.CacheFirst({cacheName:"google-fonts-webfonts",plugins:[new workbox.cacheableResponse.CacheableResponsePlugin({statuses:[0,200]}),new workbox.expiration.ExpirationPlugin({maxAgeSeconds:31536e3,maxEntries:30})]})),self.addEventListener("message",(e=>{e.data&&"SKIP_WAITING"===e.data.type&&("SKIP_WAITING"===e.data.type||console.warn(`SW: Invalid message type: ${e.data.type}`))})),workbox.precaching.precacheAndRoute([{'revision':null,'url':'/12.bundle.81eef81badc8d1242199.js'},{'revision':'ce28030fae40a3e9a7adc158449cf887','url':'/12.bundle.81eef81badc8d1242199.js.map'},{'revision':null,'url':'/125.bundle.629517ff1af1a958ddc9.js'},{'revision':'80ad2dad4489297c12a2bea64a7aa915','url':'/125.bundle.629517ff1af1a958ddc9.js.map'},{'revision':null,'url':'/181.bundle.720bf11ebdae9515747b.js'},{'revision':'8be69d634cdb6dcbea73e49bd0281e4b','url':'/181.bundle.720bf11ebdae9515747b.js.map'},{'revision':'3ec5d244dfe2973e9ba32c3a624544f9','url':'/181.css'},{'revision':'4747d22c032f0c7596cf10c94fa35141','url':'/181.css.map'},{'revision':null,'url':'/19.bundle.97cd1d5f412be83022cf.js'},{'revision':'163303b9b715840ae78040b96047ef65','url':'/19.bundle.97cd1d5f412be83022cf.js.map'},{'revision':'a37c67d7435f6a3306cfb08caed030c0','url':'/19.css'},{'revision':'81b65bda5fde922eebf4e4eadd80179c','url':'/19.css.map'},{'revision':null,'url':'/202.bundle.358aa5cd5419f9459a04.js'},{'revision':'3df54bba2137ec524f3fb39f2c61461a','url':'/202.bundle.358aa5cd5419f9459a04.js.LICENSE.txt'},{'revision':'ac4f8f8636b4ca39056b696e5fad2b23','url':'/202.bundle.358aa5cd5419f9459a04.js.map'},{'revision':null,'url':'/220.bundle.d0b0df6e3678c751fa3c.js'},{'revision':'ab19565c3c271f425eace54a300bccb4','url':'/220.bundle.d0b0df6e3678c751fa3c.js.LICENSE.txt'},{'revision':'eec45355fb47bea57d31c6255d8f6c1a','url':'/220.bundle.d0b0df6e3678c751fa3c.js.map'},{'revision':null,'url':'/221.bundle.d903e046b5d45ed1fd7a.js'},{'revision':'eccbde0a23fc93ec3b223b720a68ec40','url':'/221.bundle.d903e046b5d45ed1fd7a.js.map'},{'revision':'3aceca3ac01f28f63ee404c61a561302','url':'/221.css'},{'revision':'caf0bfe9a245ea82f2c3691437342565','url':'/221.css.map'},{'revision':null,'url':'/23.bundle.8fdf916770f44e0ecae2.js'},{'revision':'aa8ffe61dab6ddb389a92369ecda274d','url':'/23.bundle.8fdf916770f44e0ecae2.js.map'},{'revision':null,'url':'/236.bundle.5c7a2732831ec4bc80eb.js'},{'revision':'e20c2aa8e7e05977e3c59680be5d009c','url':'/236.bundle.5c7a2732831ec4bc80eb.js.map'},{'revision':null,'url':'/250.bundle.a0cf445a5802ef69d2fa.js'},{'revision':'83ea7f39f094d7f225ad2ff296fe2840','url':'/250.bundle.a0cf445a5802ef69d2fa.js.map'},{'revision':'fae0898b7f424a92fe03176f870d75a7','url':'/250.css'},{'revision':'2ecc1af100452e5b94d750be89d2180d','url':'/250.css.map'},{'revision':null,'url':'/281.bundle.17478b3e1d9df5f9a026.js'},{'revision':'778862d572bf5ca5b264f3ecd6c71cee','url':'/281.bundle.17478b3e1d9df5f9a026.js.map'},{'revision':null,'url':'/342.bundle.352b0a5b0103bd979889.js'},{'revision':'1fbd84dfb99349f7a938321c0955955a','url':'/342.bundle.352b0a5b0103bd979889.js.map'},{'revision':null,'url':'/359.bundle.e32228f9015077353eac.js'},{'revision':'77f8d5caad68561e390d7dc75f76c391','url':'/359.bundle.e32228f9015077353eac.js.map'},{'revision':'c4ea120c6da08aa75348edfa3e57ece9','url':'/36785fbd89b0e17f6099.wasm'},{'revision':null,'url':'/370.bundle.42f05adb02eaf4f41567.js'},{'revision':'3ab0a5788923f6b66ed361502ed7c40f','url':'/370.bundle.42f05adb02eaf4f41567.js.map'},{'revision':null,'url':'/410.bundle.efb50f7d564a9474c3d3.js'},{'revision':'04d4da85727a7331cf5e66da8eed5864','url':'/410.bundle.efb50f7d564a9474c3d3.js.map'},{'revision':null,'url':'/417.bundle.0ec0dae1d39259a03193.js'},{'revision':'90d558004ce3a8c56364eec2fad1a143','url':'/417.bundle.0ec0dae1d39259a03193.js.map'},{'revision':null,'url':'/451.bundle.44a0aaa91f1e65ee8fa2.js'},{'revision':'3dcbbc95d49d04190714873132a8672e','url':'/451.bundle.44a0aaa91f1e65ee8fa2.js.map'},{'revision':null,'url':'/471.bundle.bafce1ad27e0bc5c8db5.js'},{'revision':'79ac0f1ece3bbefba7467a1b848053df','url':'/471.bundle.bafce1ad27e0bc5c8db5.js.map'},{'revision':'c377e1f5fe4a207d270c3f7a8dd3e3ca','url':'/5004fdc02f329ce53b69.wasm'},{'revision':null,'url':'/506.bundle.e497ce6e8958ced779c8.js'},{'revision':'028b3ff9e987b492355c1b7fa56ae357','url':'/506.bundle.e497ce6e8958ced779c8.js.map'},{'revision':null,'url':'/530.bundle.b5e992d674170dee581a.js'},{'revision':'d18d5005c9724f255f1367655b06f0c5','url':'/530.bundle.b5e992d674170dee581a.js.LICENSE.txt'},{'revision':'7e41a2739dba88ff5326a36c98ff7f8b','url':'/530.bundle.b5e992d674170dee581a.js.map'},{'revision':'c8bd83bb3850741e0139036d4f0d8754','url':'/579.css'},{'revision':'cfd650980d83f295a12eb2987edef74d','url':'/579.css.map'},{'revision':null,'url':'/604.bundle.9477bed4d89c962cb3df.js'},{'revision':'cd6c925fd44d3eb01c4b35f8a08469e6','url':'/604.bundle.9477bed4d89c962cb3df.js.map'},{'revision':'adfcdf177b2a25b4861c65ec3055f98b','url':'/610.min.worker.js'},{'revision':'3c2206525c18cd87dd28082949a4e43e','url':'/610.min.worker.js.map'},{'revision':null,'url':'/613.bundle.5804a80c89dddd4251d0.js'},{'revision':'f4b6a943832001efc6a51be6900f8cd9','url':'/613.bundle.5804a80c89dddd4251d0.js.map'},{'revision':'5800265b6831396572fb5d32c6bd8eef','url':'/62ab5d58a2bea7b5a1dc.wasm'},{'revision':'ce10eced3ce34e663d86569b27f5bffb','url':'/65916ef3def695744bda.wasm'},{'revision':null,'url':'/663.bundle.28bd520531024fa11845.js'},{'revision':'6527ec0b3c327f007a8562af1a7e9768','url':'/663.bundle.28bd520531024fa11845.js.map'},{'revision':null,'url':'/686.bundle.9b93df830edb822372a0.js'},{'revision':'c85e3c724207122762223831410090ea','url':'/686.bundle.9b93df830edb822372a0.js.map'},{'revision':null,'url':'/687.bundle.fcd0488536d96bb682e5.js'},{'revision':'ea58b31a0d97aadcf9cba72ae2e05496','url':'/687.bundle.fcd0488536d96bb682e5.js.map'},{'revision':null,'url':'/743.bundle.58a76ef98d0f4120f602.js'},{'revision':'4e0e34f265fae8f33b01b27ae29d9d6f','url':'/743.bundle.58a76ef98d0f4120f602.js.LICENSE.txt'},{'revision':'9b52f474ff77e735ef68681498bc7a87','url':'/743.bundle.58a76ef98d0f4120f602.js.map'},{'revision':null,'url':'/757.bundle.26ceda521b4376726a11.js'},{'revision':'2f5e7935cfd071e3e2fdf5ea5c6b6f25','url':'/757.bundle.26ceda521b4376726a11.js.map'},{'revision':'cf3e4d4fa8884275461c195421812256','url':'/75788f12450d4c5ed494.wasm'},{'revision':'cc4a3a4da4ac1b863a714f93c66c6ef2','url':'/75a0c2dfe07b824c7d21.wasm'},{'revision':null,'url':'/774.bundle.0171646462dc3c8311f6.js'},{'revision':'0e9b0c785f6ab44e694934571d764d58','url':'/774.bundle.0171646462dc3c8311f6.js.LICENSE.txt'},{'revision':'3ad890f8731370d04cf8ac0431aa64ef','url':'/774.bundle.0171646462dc3c8311f6.js.map'},{'revision':null,'url':'/775.bundle.f8f6f70fabcf5cbbd7db.js'},{'revision':'a43adbe15fbbf09e693ed665d9929bdf','url':'/775.bundle.f8f6f70fabcf5cbbd7db.js.map'},{'revision':null,'url':'/788.bundle.6c391afaa3d874b45c88.js'},{'revision':'73ccf0422c7c4cb6ccae051328a85fa6','url':'/788.bundle.6c391afaa3d874b45c88.js.map'},{'revision':null,'url':'/814.bundle.98cb45449347c08563de.js'},{'revision':'47009c41335759184a0f2270ac6e6645','url':'/814.bundle.98cb45449347c08563de.js.map'},{'revision':null,'url':'/82.bundle.7ecb6591d92092b20e6e.js'},{'revision':'3d8c78958ab0ae5994867e1b36d63899','url':'/82.bundle.7ecb6591d92092b20e6e.js.map'},{'revision':'0bd71f708a33ae4bd6d8e8dec511ef15','url':'/82.css'},{'revision':'61465f96cea0a98ec641866f3934fff8','url':'/82.css.map'},{'revision':null,'url':'/822.bundle.c7db86db8d8ed49ef794.js'},{'revision':'c67f30955944e30999a3ace794a3597a','url':'/822.bundle.c7db86db8d8ed49ef794.js.map'},{'revision':null,'url':'/886.bundle.27041a87e64d23dd3de0.js'},{'revision':'bbb55579aa2f7789f8444d4aa09aee1c','url':'/886.bundle.27041a87e64d23dd3de0.js.map'},{'revision':'30ca7c265a7fdd034b427b49882e69c9','url':'/945.min.worker.js'},{'revision':'cdf6f0457d4af2cef04fc41816241bc1','url':'/945.min.worker.js.map'},{'revision':null,'url':'/957.bundle.dd46b7a4ddd3e6a28a17.js'},{'revision':'ba2c86f52ea091e7bb7474e23553c8a5','url':'/957.bundle.dd46b7a4ddd3e6a28a17.js.LICENSE.txt'},{'revision':'9c5f83e4062c7c6ece38f1f371c55d04','url':'/957.bundle.dd46b7a4ddd3e6a28a17.js.map'},{'revision':null,'url':'/99.bundle.365e02f4993598c9929f.js'},{'revision':'10bad0a26df9ba035ffd1589f08a870b','url':'/99.bundle.365e02f4993598c9929f.js.map'},{'revision':'5dcd0ebd317213406ab3ce2a4edaff37','url':'/_headers'},{'revision':'52ba15caf05a85cdb1705e818eb02299','url':'/_redirects'},{'revision':'82d9100b92aa2dfb237ac82f33689511','url':'/app-config.js'},{'revision':null,'url':'/app.bundle.2382ea06dce0989a7424.js'},{'revision':'6e0078c3007dba43752cfd0bee98f0f8','url':'/app.bundle.2382ea06dce0989a7424.js.LICENSE.txt'},{'revision':'75857755e4bab478e72b6ce66790c5ff','url':'/app.bundle.2382ea06dce0989a7424.js.map'},{'revision':'5faf45d75091663f5b5911cdd5ed01c2','url':'/app.bundle.css'},{'revision':'5804379bbba247f9407c27a0c208d31c','url':'/app.bundle.css.map'},{'revision':'cb4f64534cdf8dd88f1d7219d44490db','url':'/assets/android-chrome-144x144.png'},{'revision':'5cde390de8a619ebe55a669d2ac3effd','url':'/assets/android-chrome-192x192.png'},{'revision':'e7466a67e90471de05401e53b8fe20be','url':'/assets/android-chrome-256x256.png'},{'revision':'9bbe9b80156e930d19a4e1725aa9ddae','url':'/assets/android-chrome-36x36.png'},{'revision':'5698b2ac0c82fe06d84521fc5482df04','url':'/assets/android-chrome-384x384.png'},{'revision':'56bef3fceec344d9747f8abe9c0bba27','url':'/assets/android-chrome-48x48.png'},{'revision':'3e8b8a01290992e82c242557417b0596','url':'/assets/android-chrome-512x512.png'},{'revision':'517925e91e2ce724432d296b687d25e2','url':'/assets/android-chrome-72x72.png'},{'revision':'4c3289bc690f8519012686888e08da71','url':'/assets/android-chrome-96x96.png'},{'revision':'cf464289183184df09292f581df0fb4f','url':'/assets/apple-touch-icon-1024x1024.png'},{'revision':'0857c5282c594e4900e8b31e3bade912','url':'/assets/apple-touch-icon-114x114.png'},{'revision':'4208f41a28130a67e9392a9dfcee6011','url':'/assets/apple-touch-icon-120x120.png'},{'revision':'cb4f64534cdf8dd88f1d7219d44490db','url':'/assets/apple-touch-icon-144x144.png'},{'revision':'977d293982af7e9064ba20806b45cf35','url':'/assets/apple-touch-icon-152x152.png'},{'revision':'6de91b4d2a30600b410758405cb567b4','url':'/assets/apple-touch-icon-167x167.png'},{'revision':'87bff140e3773bd7479a620501c4aa5c','url':'/assets/apple-touch-icon-180x180.png'},{'revision':'647386c34e75f1213830ea9a38913525','url':'/assets/apple-touch-icon-57x57.png'},{'revision':'0c200fe83953738b330ea431083e7a86','url':'/assets/apple-touch-icon-60x60.png'},{'revision':'517925e91e2ce724432d296b687d25e2','url':'/assets/apple-touch-icon-72x72.png'},{'revision':'c9989a807bb18633f6dcf254b5b56124','url':'/assets/apple-touch-icon-76x76.png'},{'revision':'87bff140e3773bd7479a620501c4aa5c','url':'/assets/apple-touch-icon-precomposed.png'},{'revision':'87bff140e3773bd7479a620501c4aa5c','url':'/assets/apple-touch-icon.png'},{'revision':'05fa74ea9c1c0c3931ba96467999081d','url':'/assets/apple-touch-startup-image-1182x2208.png'},{'revision':'9e2cd03e1e6fd0520eea6846f4278018','url':'/assets/apple-touch-startup-image-1242x2148.png'},{'revision':'5591e3a1822cbc8439b99c1a40d53425','url':'/assets/apple-touch-startup-image-1496x2048.png'},{'revision':'337de578c5ca04bd7d2be19d24d83821','url':'/assets/apple-touch-startup-image-1536x2008.png'},{'revision':'cafb4ab4eafe6ef946bd229a1d88e7de','url':'/assets/apple-touch-startup-image-320x460.png'},{'revision':'d9bb9e558d729eeac5efb8be8d6111cc','url':'/assets/apple-touch-startup-image-640x1096.png'},{'revision':'038b5b02bac8b82444bf9a87602ac216','url':'/assets/apple-touch-startup-image-640x920.png'},{'revision':'2177076eb07b1d64d663d7c03268be00','url':'/assets/apple-touch-startup-image-748x1024.png'},{'revision':'4fc097443815fe92503584c4bd73c630','url':'/assets/apple-touch-startup-image-750x1294.png'},{'revision':'2e29914062dce5c5141ab47eea2fc5d9','url':'/assets/apple-touch-startup-image-768x1004.png'},{'revision':'87e13104edd0b8f21d90d3fbf3b5787c','url':'/assets/browserconfig.xml'},{'revision':'f3d9a3b647853c45b0e132e4acd0cc4a','url':'/assets/coast-228x228.png'},{'revision':'ad6e1def5c66193d649a31474bbfe45d','url':'/assets/favicon-16x16.png'},{'revision':'84d1dcdb6cdfa55e2f46be0c80fa5698','url':'/assets/favicon-32x32.png'},{'revision':'95fb44c4998a46109e49d724c060db24','url':'/assets/favicon.ico'},{'revision':'5df2a5b0cee399ac0bc40af74ba3c2cb','url':'/assets/firefox_app_128x128.png'},{'revision':'11fd9098c4b07c8a07e1d2a1e309e046','url':'/assets/firefox_app_512x512.png'},{'revision':'27cddfc922dca3bfa27b4a00fc2f5e36','url':'/assets/firefox_app_60x60.png'},{'revision':'a7a41fc30c2efc792a02c2f3a3e32dbd','url':'/assets/manifest.webapp'},{'revision':'cb4f64534cdf8dd88f1d7219d44490db','url':'/assets/mstile-144x144.png'},{'revision':'334895225e16a7777e45d81964725a97','url':'/assets/mstile-150x150.png'},{'revision':'e295cca4af6ed0365cf7b014d91b0e9d','url':'/assets/mstile-310x150.png'},{'revision':'cbefa8c42250e5f2443819fe2c69d91e','url':'/assets/mstile-310x310.png'},{'revision':'aa411a69df2b33a1362fa38d1257fa9d','url':'/assets/mstile-70x70.png'},{'revision':'5609af4f69e40e33471aee770ea1d802','url':'/assets/yandex-browser-50x50.png'},{'revision':'4b38735d858e92563557a68a72b56aee','url':'/assets/yandex-browser-manifest.json'},{'revision':'52b9a07fe0541fe8c313d9788550bf51','url':'/b6b803111e2d06a825bd.wasm'},{'revision':'7edb59d2be7c993050cb31ded36afa31','url':'/c22b37c3488e1d6c3aa4.wasm'},{'revision':'f02ae819f1cb3aaaa82eaf25cd3dd53b','url':'/cornerstoneDICOMImageLoader.min.js'},{'revision':'346733bc702ee77bf7243351d99974f8','url':'/cornerstoneDICOMImageLoader.min.js.map'},{'revision':null,'url':'/dicom-microscopy-viewer.bundle.891b86d0c0df9ca3fe6b.js'},{'revision':'a32734d2bcb762bc2576581869d2a32c','url':'/dicom-microscopy-viewer.bundle.891b86d0c0df9ca3fe6b.js.LICENSE.txt'},{'revision':'ba58b17afc3eedb4b0684bda8eb8b3f3','url':'/dicom-microscopy-viewer.bundle.891b86d0c0df9ca3fe6b.js.map'},{'revision':'fa4dc6d260154109a901a1ac672bd6d2','url':'/dicomMicroscopyViewer.min.js'},{'revision':'450494c199cf8dd8e8c34d5e98bf5334','url':'/dicomMicroscopyViewer.min.js.LICENSE.txt'},{'revision':'8a01f4e4374adc87eb07850f350aea8f','url':'/es6-shim.min.js'},{'revision':'020680fc0de257a26ef6c1df902f8d8f','url':'/es6-shim.min.js.LICENSE.txt'},{'revision':'e6d1707b32d2dee763af9be4012050a7','url':'/google.js'},{'revision':'98c3e411231a31fcaf29c56edd63d1c3','url':'/index.html'},{'revision':'2c706eca5d5ca74fcddc81692e392176','url':'/index.worker.e62ecca63f1a2e124230.worker.js'},{'revision':'beaf37c564436e46bbcd825f0330cdbf','url':'/index.worker.e62ecca63f1a2e124230.worker.js.map'},{'revision':'dea2eed78c84c32cf7a90d565a289883','url':'/index.worker.min.worker.js'},{'revision':'fd1116add443fee52a935df926396e0f','url':'/index.worker.min.worker.js.map'},{'revision':'96664560310999eea0795ed980d33a97','url':'/init-service-worker.js'},{'revision':'13370cf6d56f65af3368b9903e3f75ea','url':'/manifest.json'},{'revision':'c1bb6bd2197f670f76edc5080299fff2','url':'/ohif-logo-light.svg'},{'revision':'f3fdea69420c62b41b407fb2aadff911','url':'/ohif-logo.svg'},{'revision':'8032232e4e08184ee8a9e4c018c8ba55','url':'/oidc-client.min.js'},{'revision':'4b43be1f14657780d4b120e50d8fee65','url':'/oidc-client.min.js.LICENSE.txt'},{'revision':'f5fd3850f3da362de535533e3803383f','url':'/polyfill.min.js'},{'revision':'e5242fadf304e9916f57adeddd642fa2','url':'/silent-refresh.html'},{'revision':'86de03f134695628272fe13ad7967ef0','url':'/sw.js.map'}]);
//# sourceMappingURL=sw.js.map