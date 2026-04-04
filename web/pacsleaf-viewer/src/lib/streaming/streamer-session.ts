import type { StreamingQuality } from '../runtime-config'

export interface StreamSessionRequest {
  studyUid: string
  seriesUid: string
  quality: StreamingQuality
  layout?: 'single' | 'quad'
  supportsVideoDecoder?: boolean
}

export interface WebTransportConnectInfo {
  url: string
  certificateHash: number[]
}

export interface StreamSessionResponse {
  sessionId: string
  websocketUrl: string
  webtransport?: WebTransportConnectInfo
  transport: 'websocket' | 'webtransport'
  quality: StreamingQuality
}

function normalizeStreamerUrl(value: string): string {
  return value.trim().replace(/\/+$/, '')
}

let supportsVideoDecoderPromise: Promise<boolean> | undefined

export function supportsStreamingVideoDecoder(): Promise<boolean> {
  if (!supportsVideoDecoderPromise) {
    supportsVideoDecoderPromise = detectStreamingVideoDecoderSupport()
  }

  return supportsVideoDecoderPromise
}

async function detectStreamingVideoDecoderSupport(): Promise<boolean> {
  if (typeof VideoDecoder === 'undefined' || typeof EncodedVideoChunk === 'undefined') {
    return false
  }

  try {
    const support = await VideoDecoder.isConfigSupported({
      codec: 'hvc1.1.6.L93.B0',
      codedWidth: 1280,
      codedHeight: 720,
      hardwareAcceleration: 'prefer-hardware',
      optimizeForLatency: true,
    })
    return Boolean(support.supported)
  } catch {
    return false
  }
}

async function readErrorMessage(response: Response): Promise<string> {
  const message = (await response.text()).trim()
  return message || `Streamer request failed with status ${response.status}`
}

export async function createStreamSession(
  streamerUrl: string,
  request: StreamSessionRequest,
): Promise<StreamSessionResponse> {
  const response = await fetch(`${normalizeStreamerUrl(streamerUrl)}/control/session`, {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
    },
    body: JSON.stringify(request),
  })

  if (!response.ok) {
    throw new Error(await readErrorMessage(response))
  }

  return response.json() as Promise<StreamSessionResponse>
}

export async function deleteStreamSession(
  streamerUrl: string,
  sessionId: string,
): Promise<void> {
  const response = await fetch(
    `${normalizeStreamerUrl(streamerUrl)}/control/session/${encodeURIComponent(sessionId)}`,
    {
      method: 'DELETE',
    },
  )

  if (!response.ok && response.status !== 404) {
    throw new Error(await readErrorMessage(response))
  }
}
