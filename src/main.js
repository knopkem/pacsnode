// Modules to control application life and create native browser window
const { app, shell, BrowserWindow } = require('electron')
const path = require('path')
const { electronApp, optimizer } = require('@electron-toolkit/utils')
const server = require('fastify')({
  logger: false
})
const fastifyStatic = require('@fastify/static')
const fastifyCors = require('@fastify/cors')
const fastifySensible = require('@fastify/sensible')
const fastifyHelmet = require('@fastify/helmet')
const fastifyAutoload = require('@fastify/autoload')

const utils = require('./utils')

const logger = utils.getLogger()

server.register(fastifyStatic, {
  root: path.join(__dirname, '../resources')
})
server.setNotFoundHandler((_req, res) => {
  res.sendFile('index.html')
})
server.register(fastifyCors, {})
server.register(fastifySensible)
server.register(fastifyHelmet, {
  contentSecurityPolicy: false,
  crossOriginEmbedderPolicy: { policy: 'require-corp' },
  crossOriginResourcePolicy: { policy: 'same-site' },
  crossOriginOpenerPolicy: { policy: 'same-origin' }
})

server.register(fastifyAutoload, {
  dir: path.join(__dirname, 'routes')
})
server.register(fastifyAutoload, {
  dir: path.join(__dirname, 'routes'),
  options: { prefix: '/viewer' }
})

server.setErrorHandler(async (err) => {
  logger.error(err.message) // 'caught'
})

// log exceptions
process.on('uncaughtException', (err) => {
  logger.error('uncaught exception received:')
  logger.error(err.stack)
})

//------------------------------------------------------------------

process.on('SIGINT', async () => {
  await logger.info('shutting down web server...')
  server.close().then(
    async () => {
      await logger.info('webserver shutdown successfully')
    },
    (err) => {
      logger.error('webserver shutdown failed', err)
    }
  )
  await logger.info('shutting down DICOM SCP server...')
  await utils.shutdown()
  process.exit(1)
})

//------------------------------------------------------------------

logger.info('starting...')
server.listen({ port: 9876, host: '127.0.0.1' }, async (err, address) => {
  if (err) {
    await logger.error(err, address)
    process.exit(1)
  }
  utils.startScp()
  utils.sendEcho()
})

//------------------------------------------------------------------

function createWindow() {
  // Create the browser window.
  const mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    show: false,
    autoHideMenuBar: true,
    ...(process.platform === 'linux'
      ? {
          icon: path.join(__dirname, '../resources/icon.png')
        }
      : {}),
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      sandbox: false
    }
  })

  mainWindow.on('ready-to-show', () => {
    mainWindow.show()
  })

  mainWindow.webContents.setWindowOpenHandler((details) => {
    shell.openExternal(details.url)
    return { action: 'deny' }
  })

  // and load the index.html of the app.
  //mainWindow.loadFile(path.join(__dirname, 'index.html'))
  mainWindow.loadURL('http://localhost:9876')
}

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.whenReady().then(() => {
  // Set app user model id for windows
  electronApp.setAppUserModelId('com.electron')

  // Default open or close DevTools by F12 in development
  // and ignore CommandOrControl + R in production.
  // see https://github.com/alex8088/electron-toolkit/tree/master/packages/utils
  app.on('browser-window-created', (_, window) => {
    optimizer.watchWindowShortcuts(window)
  })

  createWindow()

  app.on('activate', function () {
    // On macOS it's common to re-create a window in the app when the
    // dock icon is clicked and there are no other windows open.
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

// Quit when all windows are closed, except on macOS. There, it's common
// for applications and their menu bar to stay active until the user quits
// explicitly with Cmd + Q.
app.on('window-all-closed', function () {
  if (process.platform !== 'darwin') {
    server.close()
    utils.shutdown().then(() => {
      app.quit()
    })
  }
})

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and require them here.
