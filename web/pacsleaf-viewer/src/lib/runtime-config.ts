export type RenderingMode = 'client' | 'streaming'
export type StreamingQuality = 'diagnostic' | 'balanced' | 'mobile'

export interface PacsleafRuntimeConfig {
  appName: string
  routerBasename: string
  routes: {
    studies: string
    viewer: string
    settings: string
  }
  dicomweb: {
    qidoRoot: string
    wadoRoot: string
    wadoUriRoot: string
  }
  restApiRoot: string
  rendering: {
    defaultMode: RenderingMode
  }
  streaming: {
    defaultUrl: string
    defaultQuality: StreamingQuality
  }
  viewer: {
    autoSelectFirstSeries: boolean
    showMetadataRail: boolean
  }
  server: {
    aeTitle: string
    httpPort: number
    dicomPort: number
    version: string
  }
}

declare global {
  interface Window {
    __PACSLEAF_CONFIG__?: PacsleafRuntimeConfig
  }
}

const fallbackConfig: PacsleafRuntimeConfig = {
  appName: 'pacsleaf',
  routerBasename: '/viewer',
  routes: {
    studies: '/studies',
    viewer: '/viewer/:studyUid',
    settings: '/settings',
  },
  dicomweb: {
    qidoRoot: '/wado',
    wadoRoot: '/wado',
    wadoUriRoot: '/wado',
  },
  restApiRoot: '/api',
  rendering: {
    defaultMode: 'streaming',
  },
  streaming: {
    defaultUrl: `http://${window.location.hostname}:43120`,
    defaultQuality: 'balanced',
  },
  viewer: {
    autoSelectFirstSeries: true,
    showMetadataRail: true,
  },
  server: {
    aeTitle: 'PACSNODE',
    httpPort: 8042,
    dicomPort: 4242,
    version: 'dev',
  },
}

let runtimeConfigPromise: Promise<void> | undefined

export async function ensureRuntimeConfigLoaded(): Promise<void> {
  if (window.__PACSLEAF_CONFIG__) {
    return
  }

  if (!runtimeConfigPromise) {
    runtimeConfigPromise = new Promise((resolve, reject) => {
      const runtimeScript = document.createElement('script')
      runtimeScript.src = new URL(/* @vite-ignore */ '../app-config.js', import.meta.url).toString()
      runtimeScript.async = false
      runtimeScript.onload = () => resolve()
      runtimeScript.onerror = () =>
        reject(new Error('Unable to load the pacsleaf runtime config from app-config.js.'))
      document.head.appendChild(runtimeScript)
    })
  }

  await runtimeConfigPromise
}

export function getRuntimeConfig(): PacsleafRuntimeConfig {
  const runtimeConfig = window.__PACSLEAF_CONFIG__

  return {
    ...fallbackConfig,
    ...runtimeConfig,
    routes: {
      ...fallbackConfig.routes,
      ...(runtimeConfig?.routes ?? {}),
    },
    dicomweb: {
      ...fallbackConfig.dicomweb,
      ...(runtimeConfig?.dicomweb ?? {}),
    },
    rendering: {
      ...fallbackConfig.rendering,
      ...(runtimeConfig?.rendering ?? {}),
    },
    streaming: {
      ...fallbackConfig.streaming,
      ...(runtimeConfig?.streaming ?? {}),
    },
    viewer: {
      ...fallbackConfig.viewer,
      ...(runtimeConfig?.viewer ?? {}),
    },
    server: {
      ...fallbackConfig.server,
      ...(runtimeConfig?.server ?? {}),
    },
  }
}
