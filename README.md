# pacsnode

A minimal Electron PACS Viewer application. It is basically an electron version of the dicomweb-pacs node based server.
Web DICOM Viewer: [OHIF Viewer](https://github.com/OHIF/Viewers) V3.7.0
Store-SCP: DCMTK + sqlite
Params: 
      * AET: "DICOMWEB_PACS",
      * IP: "127.0.0.1",
      * Port: "8888"

Roadmap:
* admin panel to manage stored data and configuration
* better logging

## Recommended IDE Setup

- [VSCode](https://code.visualstudio.com/) + [ESLint](https://marketplace.visualstudio.com/items?itemName=dbaeumer.vscode-eslint) + [Prettier](https://marketplace.visualstudio.com/items?itemName=esbenp.prettier-vscode)

## Project Setup

### Install

```bash
$ npm install
```

### Development

```bash
$ npm run dev
```

### Build

```bash
# For windows
$ npm run build:win

# For macOS
$ npm run build:mac

# For Linux
$ npm run build:linux
```
