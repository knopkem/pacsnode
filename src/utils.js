const dict = require('dicom-data-dictionary')
const dimse = require('dicom-dimse-native')
const dict2 = require('@iwharris/dicom-data-dictionary')
const fs = require('fs')
const shell = require('shelljs')
const logger = require('electron-log/main')

// make sure default directories exist
shell.mkdir('-p', './data')


const source = {
  aet: 'DICOMWEB_PACS',
  ip: '127.0.0.1',
  port: '8888'
}
const peers = [
  {
    aet: 'CONQUESTSRV1',
    ip: '127.0.0.1',
    port: '5678'
  }
]

//------------------------------------------------------------------

const findDicomName = (name) => {
  // eslint-disable-next-line no-restricted-syntax
  for (const key of Object.keys(dict.standardDataElements)) {
    const value = dict.standardDataElements[key]
    if (value.name === name || name === key) {
      return key
    }
  }
  return undefined
}

const findVR = (name) => {
  const dataElement = dict2.get_element(name)
  if (dataElement) {
    return dataElement.vr
  }
  return ''
}

//------------------------------------------------------------------

const utils = {
  getLogger: () => logger,
  startScp: () => {
    const ts = '1.2.840.10008.1.2';
    const j = {
      source,
      target: source,
      peers,
      verbose: false,
      storagePath: './data',
      permissive: true,
      netTransferPrefer: ts,
      netTransferPropose: ts,
      writeTransfer: ts,

    }
    j.peers.push(j.source)

    logger.info(`pacs-server listening on port: ${j.source.port}`)

    dimse.startStoreScp(j, (result) => {
      // currently this will never finish
      logger.info(JSON.parse(result))
    })
  },
  shutdown: () => {
    const j = {
      source,
      target: source,
      verbose: false
    }

    logger.info(`sending shutdown request to target: ${j.target.aet}`)

    return new Promise((resolve, reject) => {
      dimse.shutdownScu(j, (result) => {
        if (result && result.length > 0) {
          try {
            logger.info(JSON.parse(result))
            resolve()
          } catch (error) {
            logger.error(result)
            reject()
          }
        }
        reject()
      })
    })
  },
  sendEcho: () => {
    const j = {
      source,
      target: source,
      verbose: false
    }

    logger.info(`sending C-ECHO to target: ${j.target.aet}`)

    return new Promise((resolve, reject) => {
      dimse.echoScu(j, (result) => {
        if (result && result.length > 0) {
          try {
            logger.info(JSON.parse(result))
            resolve()
          } catch (error) {
            logger.error(result)
            reject()
          }
        }
        reject()
      })
    })
  },
  fileExists: (pathname) =>
    new Promise((resolve, reject) => {
      fs.access(pathname, (err) => {
        if (err) {
          reject(err)
        } else {
          resolve()
        }
      })
    }),
  studyLevelTags: () => [
    '00080005',
    '00080020',
    '00080030',
    '00080050',
    '00080054',
    '00080056',
    '00080061',
    '00080090',
    '00081190',
    '00100010',
    '00100020',
    '00100030',
    '00100040',
    '0020000D',
    '00200010',
    '00201206',
    '00201208'
  ],
  seriesLevelTags: () => [
    '00080005',
    '00080054',
    '00080056',
    '00080060',
    '0008103E',
    '00081190',
    '0020000E',
    '00200011',
    '00201209'
  ],
  imageLevelTags: () => ['00080016', '00080018'],
  imageMetadataTags: () => [
    '00080016',
    '00080018',
    '00080060',
    '00280002',
    '00280004',
    '00280010',
    '00280011',
    '00280030',
    '00280100',
    '00280101',
    '00280102',
    '00280103',
    '00281050',
    '00281051',
    '00281052',
    '00281053',
    '00200032',
    '00200037'
  ],
  compressFile: (inputFile, outputDirectory, transferSyntax) => {
    const j = {
      sourcePath: inputFile,
      storagePath: outputDirectory,
      writeTransfer: transferSyntax || '1.2.840.10008.1.2',
      verbose: false,
      enableRecompression: true
    }
    return new Promise((resolve, reject) => {
      dimse.recompress(j, (result) => {
        if (result && result.length > 0) {
          try {
            const json = JSON.parse(result)
            if (json.code === 0) {
              resolve()
            } else {
              logger.error(`recompression failure (${inputFile}): ${json.message}`)
              reject()
            }
          } catch (error) {
            logger.error(error)
            logger.error(result)
            reject()
          }
        } else {
          logger.error('invalid result received')
          reject()
        }
      })
    })
  },
  doFind: (queryLevel, query, defaults) => {
    // add query retrieve level
    const j = {
      source,
      target: source,
      verbose: false,
      tags: [
        {
          key: '00080052',
          value: queryLevel
        }
      ]
    }

    // parse all include fields
    const includes = query.includefield

    let tags = []
    if (includes) {
      tags = includes.split(',')
    }
    tags.push(...defaults)

    // add parsed tags
    tags.forEach((element) => {
      const tagName = findDicomName(element) || element
      j.tags.push({ key: tagName, value: '' })
    })

    // add search param
    let invalidInput = false
    const minCharsQido = 0;
    Object.keys(query).forEach((propName) => {
      const tag = findDicomName(propName)
      const vr = findVR(propName)
      if (tag) {
        let v = query[propName]
        // string vr types check
        if (['PN', 'LO', 'LT', 'SH', 'ST'].includes(vr)) {
          // just make sure to remove any wildcards from prefix and suffix
          v = v.replace(/^[*]/, '')
          v = v.replace(/[*]$/, '')

          // check if minimum number of chars are reached from input
          if (minCharsQido > v.length) {
            invalidInput = true
          }
          // auto append wildcard
          v += '*'
        }
        j.tags.push({ key: tag, value: v })
      }
    })

    if (invalidInput) {
      return []
    }

    const offset = query.offset ? parseInt(query.offset, 10) : 0

    // run find scu and return json response
    return new Promise((resolve) => {
      dimse.findScu(j, (result) => {
        if (result && result.length > 0) {
          try {
            const json = JSON.parse(result)
            if (json.code === 0) {
              const container = JSON.parse(json.container)
              if (container) {
                resolve(container.slice(offset))
              } else {
                resolve([])
              }
            } else if (json.code === 1) {
              logger.info('query is pending...')
            } else {
              logger.error(`c-find failure: ${json.message}`)
              resolve([])
            }
          } catch (error) {
            logger.error(error)
            logger.error(result)
            resolve([])
          }
        } else {
          logger.error('invalid result received')
          resolve([])
        }
      })
    })
  }
}
module.exports = utils
