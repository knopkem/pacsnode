import type { WebTransportConnectInfo } from './streamer-session'

const FRAME_PACKET_HEADER_SIZE = 18
const TAG_POINTER_MOVE = 0x01
const TAG_POINTER_DOWN = 0x02
const TAG_POINTER_UP = 0x03
const TAG_SCROLL = 0x04
const TAG_KEY_DOWN = 0x10
const TAG_KEY_UP = 0x11

export type StreamTransportKind = 'websocket' | 'webtransport'

export interface StreamControlMessage {
  type: string
  message?: string
  transportRttMs?: number
  encodeTimeUs?: number
  codec?: string
  hardwareAcceleration?: string
  optimizeForLatency?: boolean
  codedWidth?: number
  codedHeight?: number
  descriptionBase64?: string
}

interface FramePacket {
  frameId: number
  fragmentIndex: number
  fragmentCount: number
  timestampUs: number
  isKeyframe: boolean
  isRefine: boolean
  isLossless: boolean
  payload: Uint8Array
}

interface FrameAccumulator {
  fragmentCount: number
  received: number
  timestampUs: number
  isKeyframe: boolean
  isRefine: boolean
  isLossless: boolean
  fragments: Array<Uint8Array | undefined>
}

interface CompletedFrame {
  timestampUs: number
  isKeyframe: boolean
  isRefine: boolean
  isLossless: boolean
  payload: Uint8Array
}

interface CanvasTransportOptions {
  websocketUrl: string
  webtransport?: WebTransportConnectInfo
  canvas: HTMLCanvasElement
  onStatus?: (message: string) => void
  onControlMessage?: (message: StreamControlMessage) => void
  onTransportChange?: (transport: StreamTransportKind) => void
}

function appendFloat32(buffer: ArrayBuffer, offset: number, value: number) {
  new DataView(buffer).setFloat32(offset, value, true)
}

function supportsWebTransport(): boolean {
  return typeof WebTransport !== 'undefined'
}

function isReliableInputTag(tag: number): boolean {
  return tag === TAG_POINTER_DOWN || tag === TAG_POINTER_UP || tag === TAG_KEY_DOWN || tag === TAG_KEY_UP
}

function cloneBytes(data: ArrayBuffer | Uint8Array): Uint8Array {
  if (data instanceof Uint8Array) {
    return new Uint8Array(data)
  }

  return new Uint8Array(data.slice(0))
}

function copyToArrayBuffer(data: Uint8Array): ArrayBuffer {
  const copied = new Uint8Array(data.byteLength)
  copied.set(data)
  return copied.buffer
}

function decodeUtf8(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes)
}

export class CanvasTransportClient {
  private readonly options: CanvasTransportOptions
  private socket: WebSocket | undefined
  private webTransport: WebTransport | undefined
  private webTransportDatagramWriter:
    | WritableStreamDefaultWriter<Uint8Array>
    | undefined
  private webTransportDatagramReader:
    | ReadableStreamDefaultReader<Uint8Array>
    | undefined
  private webTransportControlReader:
    | ReadableStreamDefaultReader<ReadableStream<Uint8Array>>
    | undefined
  private readonly pendingFrames = new Map<number, FrameAccumulator>()
  private readonly pendingVideoFrames: CompletedFrame[] = []
  private renderGeneration = 0
  private reliableInputChain: Promise<void> = Promise.resolve()
  private videoDecoder: VideoDecoder | undefined
  private videoDecoderConfigKey: string | undefined

  constructor(options: CanvasTransportOptions) {
    this.options = options
  }

  async connect(): Promise<void> {
    if (this.isConnected()) {
      return
    }

    let webtransportFailure: Error | undefined
    if (this.options.webtransport && supportsWebTransport()) {
      try {
        await this.connectWebTransport(this.options.webtransport)
        return
      } catch (error: unknown) {
        webtransportFailure =
          error instanceof Error ? error : new Error('The WebTransport session could not be opened.')
        this.options.onStatus?.('WebTransport unavailable, falling back to WebSocket.')
        this.resetWebTransportState()
      }
    }

    await this.connectWebSocket(webtransportFailure)
  }

  disconnect() {
    this.socket?.close()
    this.socket = undefined
    this.resetWebTransportState()
    this.closeVideoDecoder()
    this.pendingFrames.clear()
    this.pendingVideoFrames.length = 0
  }

  sendPointerMove(x: number, y: number, buttons: number, timestampMs: number) {
    const buffer = new ArrayBuffer(14)
    const view = new DataView(buffer)
    view.setUint8(0, TAG_POINTER_MOVE)
    view.setUint8(1, buttons)
    appendFloat32(buffer, 2, x)
    appendFloat32(buffer, 6, y)
    view.setUint32(10, timestampMs, true)
    this.sendBinary(buffer)
  }

  sendPointerDown(button: number, x: number, y: number) {
    const buffer = new ArrayBuffer(10)
    const view = new DataView(buffer)
    view.setUint8(0, TAG_POINTER_DOWN)
    view.setUint8(1, button)
    appendFloat32(buffer, 2, x)
    appendFloat32(buffer, 6, y)
    this.sendBinary(buffer)
  }

  sendPointerUp(button: number, x: number, y: number) {
    const buffer = new ArrayBuffer(10)
    const view = new DataView(buffer)
    view.setUint8(0, TAG_POINTER_UP)
    view.setUint8(1, button)
    appendFloat32(buffer, 2, x)
    appendFloat32(buffer, 6, y)
    this.sendBinary(buffer)
  }

  sendScroll(deltaX: number, deltaY: number, mode: 0 | 1 | 2 = 0) {
    const buffer = new ArrayBuffer(10)
    const view = new DataView(buffer)
    view.setUint8(0, TAG_SCROLL)
    appendFloat32(buffer, 1, deltaX)
    appendFloat32(buffer, 5, deltaY)
    view.setUint8(9, mode)
    this.sendBinary(buffer)
  }

  sendKeyDown(code: number) {
    this.sendBinary(new Uint8Array([TAG_KEY_DOWN, (code >> 8) & 0xff, code & 0xff]))
  }

  sendKeyUp(code: number) {
    this.sendBinary(new Uint8Array([TAG_KEY_UP, (code >> 8) & 0xff, code & 0xff]))
  }

  private isConnected(): boolean {
    return (
      this.socket?.readyState === WebSocket.OPEN ||
      this.webTransport !== undefined
    )
  }

  private async connectWebSocket(previousFailure?: Error): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      const socket = new WebSocket(this.options.websocketUrl)
      socket.binaryType = 'arraybuffer'

      socket.onopen = () => {
        this.socket = socket
        this.setActiveTransport('websocket')
        this.options.onStatus?.(
          previousFailure
            ? `WebTransport failed (${previousFailure.message}). WebSocket fallback connected.`
            : 'Streaming transport connected.',
        )
        resolve()
      }
      socket.onerror = () => reject(new Error('The streaming transport could not be opened.'))
      socket.onclose = () => {
        if (this.socket === socket) {
          this.socket = undefined
          this.options.onStatus?.('Streaming transport closed.')
        }
      }
      socket.onmessage = (event) => {
        void this.handleSocketMessage(event.data).catch((error: unknown) => {
          this.options.onStatus?.(
            error instanceof Error ? error.message : 'Unable to decode streamed frame data.',
          )
        })
      }
    })
  }

  private async connectWebTransport(connectInfo: WebTransportConnectInfo): Promise<void> {
    const transport = new WebTransport(connectInfo.url, {
      serverCertificateHashes: [
        {
          algorithm: 'sha-256',
          value: new Uint8Array(connectInfo.certificateHash),
        },
      ],
    })
    this.webTransport = transport

    await transport.ready

    this.webTransportDatagramWriter = transport.datagrams.writable.getWriter()
    this.webTransportDatagramReader = transport.datagrams.readable.getReader()
    this.webTransportControlReader = transport.incomingUnidirectionalStreams.getReader()
    this.setActiveTransport('webtransport')
    this.options.onStatus?.('Streaming transport connected.')

    void this.readWebTransportDatagrams(transport)
    void this.readWebTransportControls(transport)
    void transport.closed
      .then(() => {
        if (this.webTransport === transport) {
          this.resetWebTransportState()
          this.options.onStatus?.('Streaming transport closed.')
        }
      })
      .catch((error: unknown) => {
        if (this.webTransport === transport) {
          this.resetWebTransportState()
          this.options.onStatus?.(
            error instanceof Error
              ? `WebTransport closed: ${error.message}`
              : 'WebTransport closed unexpectedly.',
          )
        }
      })
  }

  private async readWebTransportDatagrams(transport: WebTransport): Promise<void> {
    const reader = this.webTransportDatagramReader
    if (!reader) {
      return
    }

    while (this.webTransport === transport) {
      const { done, value } = await reader.read()
      if (done || !value) {
        break
      }

      const completedFrame = this.acceptFramePacket(copyToArrayBuffer(cloneBytes(value)))
      if (completedFrame) {
        await this.drawFrame(completedFrame)
      }
    }
  }

  private async readWebTransportControls(transport: WebTransport): Promise<void> {
    const reader = this.webTransportControlReader
    if (!reader) {
      return
    }

    while (this.webTransport === transport) {
      const { done, value } = await reader.read()
      if (done || !value) {
        break
      }

      await this.readControlStream(value)
    }
  }

  private async readControlStream(stream: ReadableStream<Uint8Array>): Promise<void> {
    const reader = stream.getReader()
    const chunks: Uint8Array[] = []
    let totalLength = 0

    while (true) {
      const { done, value } = await reader.read()
      if (done) {
        break
      }

      if (value) {
        const chunk = cloneBytes(value)
        chunks.push(chunk)
        totalLength += chunk.byteLength
      }
    }

    const bytes = new Uint8Array(totalLength)
    let offset = 0
    for (const chunk of chunks) {
      bytes.set(chunk, offset)
      offset += chunk.byteLength
    }
    this.handleControlMessage(decodeUtf8(bytes))
  }

  private resetWebTransportState() {
    if (this.webTransportControlReader) {
      void this.webTransportControlReader.cancel().catch(() => undefined)
      this.webTransportControlReader.releaseLock()
      this.webTransportControlReader = undefined
    }
    if (this.webTransportDatagramReader) {
      void this.webTransportDatagramReader.cancel().catch(() => undefined)
      this.webTransportDatagramReader.releaseLock()
      this.webTransportDatagramReader = undefined
    }
    if (this.webTransportDatagramWriter) {
      void this.webTransportDatagramWriter.close().catch(() => undefined)
      this.webTransportDatagramWriter.releaseLock()
      this.webTransportDatagramWriter = undefined
    }
    this.webTransport?.close()
    this.webTransport = undefined
  }

  private setActiveTransport(transport: StreamTransportKind) {
    this.options.onTransportChange?.(transport)
  }

  private sendBinary(data: ArrayBuffer | Uint8Array) {
    const bytes = cloneBytes(data)

    if (this.webTransport && this.webTransportDatagramWriter) {
      if (isReliableInputTag(bytes[0] ?? 0)) {
        this.reliableInputChain = this.reliableInputChain
          .then(() => this.sendReliableWebTransport(bytes))
          .catch(() => undefined)
      } else {
        void this.sendUnreliableWebTransport(bytes)
      }
      return
    }

    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(bytes)
    }
  }

  private async sendUnreliableWebTransport(bytes: Uint8Array): Promise<void> {
    await this.webTransportDatagramWriter?.write(bytes)
  }

  private async sendReliableWebTransport(bytes: Uint8Array): Promise<void> {
    if (!this.webTransport) {
      return
    }

    const stream = await this.webTransport.createUnidirectionalStream()
    const writer = stream.getWriter()
    await writer.write(bytes)
    await writer.close()
  }

  private async handleSocketMessage(data: ArrayBuffer | Blob | string): Promise<void> {
    if (typeof data === 'string') {
      this.handleControlMessage(data)
      return
    }

    const buffer = data instanceof Blob ? await data.arrayBuffer() : data
    const completedFrame = this.acceptFramePacket(buffer)
    if (completedFrame) {
      await this.drawFrame(completedFrame)
    }
  }

  private handleControlMessage(text: string) {
    const parsed = JSON.parse(text) as StreamControlMessage
    this.options.onControlMessage?.(parsed)

    if (parsed.type === 'status' && typeof parsed.message === 'string') {
      this.options.onStatus?.(parsed.message)
      return
    }

    if (parsed.type === 'decoder-config') {
      void this.configureVideoDecoder(parsed).catch((error: unknown) => {
        this.options.onStatus?.(
          error instanceof Error
            ? error.message
            : 'The browser video decoder could not be configured.',
        )
      })
    }
  }

  private acceptFramePacket(buffer: ArrayBuffer): CompletedFrame | undefined {
    const packet = this.parseFramePacket(buffer)
    const accumulator = this.pendingFrames.get(packet.frameId) ?? {
      fragmentCount: packet.fragmentCount,
      received: 0,
      timestampUs: packet.timestampUs,
      isKeyframe: packet.isKeyframe,
      isRefine: packet.isRefine,
      isLossless: packet.isLossless,
      fragments: Array.from({ length: packet.fragmentCount }),
    }

    if (!accumulator.fragments[packet.fragmentIndex]) {
      accumulator.fragments[packet.fragmentIndex] = packet.payload
      accumulator.received += 1
    }

    this.pendingFrames.set(packet.frameId, accumulator)

    if (accumulator.received !== accumulator.fragmentCount) {
      return undefined
    }

    this.pendingFrames.delete(packet.frameId)

    const totalLength = accumulator.fragments.reduce(
      (length, fragment) => length + (fragment?.length ?? 0),
      0,
    )
    const output = new Uint8Array(totalLength)
    let offset = 0

    for (const fragment of accumulator.fragments) {
      if (!fragment) {
        return undefined
      }

      output.set(fragment, offset)
      offset += fragment.length
    }

    return {
      timestampUs: accumulator.timestampUs,
      isKeyframe: accumulator.isKeyframe,
      isRefine: accumulator.isRefine,
      isLossless: accumulator.isLossless,
      payload: output,
    }
  }

  private parseFramePacket(buffer: ArrayBuffer): FramePacket {
    if (buffer.byteLength < FRAME_PACKET_HEADER_SIZE) {
      throw new Error('Received an incomplete streaming frame packet.')
    }

    const view = new DataView(buffer)
    return {
      frameId: view.getUint32(0, true),
      fragmentIndex: view.getUint16(4, true),
      fragmentCount: view.getUint16(6, true),
      timestampUs: Number(view.getBigUint64(8, true)),
      isKeyframe: (view.getUint8(16) & 0x01) !== 0,
      isLossless: (view.getUint8(16) & 0x02) !== 0,
      isRefine: (view.getUint8(17) & 0x01) !== 0,
      payload: new Uint8Array(buffer.slice(FRAME_PACKET_HEADER_SIZE)),
    }
  }

  private async drawFrame(frame: CompletedFrame): Promise<void> {
    const mimeType = this.detectImageMimeType(frame.payload)
    if (mimeType) {
      await this.drawImageFrame(frame.payload, mimeType)
      return
    }

    if (this.videoDecoder) {
      this.decodeVideoFrame(frame)
      return
    }

    this.pendingVideoFrames.push(frame)
    if (this.pendingVideoFrames.length > 4) {
      this.pendingVideoFrames.shift()
    }
  }

  private async drawImageFrame(payload: Uint8Array, mimeType: string): Promise<void> {
    const blob = new Blob([copyToArrayBuffer(payload)], { type: mimeType })
    const objectUrl = URL.createObjectURL(blob)
    const image = new Image()
    const generation = ++this.renderGeneration

    try {
      await new Promise<void>((resolve, reject) => {
        image.onload = () => resolve()
        image.onerror = () => reject(new Error('The streamed frame image could not be decoded.'))
        image.src = objectUrl
      })

      if (generation !== this.renderGeneration) {
        return
      }

      const context = this.options.canvas.getContext('2d')
      if (!context) {
        throw new Error('The streaming canvas could not acquire a 2D drawing context.')
      }

      const cw = this.options.canvas.width
      const ch = this.options.canvas.height
      context.clearRect(0, 0, cw, ch)

      // Draw the frame preserving its native aspect ratio (letterbox).
      const iw = image.naturalWidth
      const ih = image.naturalHeight
      const scale = Math.min(cw / iw, ch / ih)
      const dw = Math.round(iw * scale)
      const dh = Math.round(ih * scale)
      const dx = Math.round((cw - dw) / 2)
      const dy = Math.round((ch - dh) / 2)
      context.drawImage(image, dx, dy, dw, dh)
    } finally {
      URL.revokeObjectURL(objectUrl)
    }
  }

  private decodeVideoFrame(frame: CompletedFrame) {
    if (!this.videoDecoder) {
      throw new Error('The browser video decoder is not available for streamed video frames.')
    }

    this.videoDecoder.decode(
      new EncodedVideoChunk({
        type: frame.isKeyframe ? 'key' : 'delta',
        timestamp: frame.timestampUs,
        data: frame.payload,
      }),
    )
  }

  private async configureVideoDecoder(message: StreamControlMessage): Promise<void> {
    if (
      typeof VideoDecoder === 'undefined' ||
      typeof EncodedVideoChunk === 'undefined' ||
      typeof message.codec !== 'string'
    ) {
      return
    }

    const config: VideoDecoderConfig = {
      codec: message.codec,
      hardwareAcceleration:
        (message.hardwareAcceleration as VideoDecoderConfig['hardwareAcceleration']) ??
        'prefer-hardware',
      optimizeForLatency: message.optimizeForLatency ?? true,
      codedWidth: message.codedWidth,
      codedHeight: message.codedHeight,
      description: decodeOptionalBase64(message.descriptionBase64),
    }
    const configKey = JSON.stringify({
      codec: config.codec,
      hardwareAcceleration: config.hardwareAcceleration,
      optimizeForLatency: config.optimizeForLatency,
      codedWidth: config.codedWidth ?? null,
      codedHeight: config.codedHeight ?? null,
      descriptionBase64: message.descriptionBase64 ?? null,
    })
    if (configKey === this.videoDecoderConfigKey) {
      return
    }

    const support = await VideoDecoder.isConfigSupported(config)
    if (!support.supported) {
      throw new Error(`The browser does not support streamed codec ${message.codec}.`)
    }

    if (!this.videoDecoder || this.videoDecoder.state === 'closed') {
      this.videoDecoder = new VideoDecoder({
        output: (frame) => this.drawDecodedVideoFrame(frame),
        error: (error) => {
          this.options.onStatus?.(
            error instanceof Error ? `WebCodec decode failed: ${error.message}` : 'WebCodec decode failed.',
          )
        },
      })
    }

    this.videoDecoder.configure(support.config ?? config)
    this.videoDecoderConfigKey = configKey
    this.options.onStatus?.(`Streaming decoder ready (${message.codec}).`)
    while (this.pendingVideoFrames.length) {
      const frame = this.pendingVideoFrames.shift()
      if (frame) {
        this.decodeVideoFrame(frame)
      }
    }
  }

  private drawDecodedVideoFrame(frame: VideoFrame) {
    try {
      const context = this.options.canvas.getContext('2d')
      if (!context) {
        throw new Error('The streaming canvas could not acquire a 2D drawing context.')
      }

      const cw = this.options.canvas.width
      const ch = this.options.canvas.height
      context.clearRect(0, 0, cw, ch)

      // Preserve aspect ratio (letterbox) — same as the image-frame path.
      const iw = frame.displayWidth
      const ih = frame.displayHeight
      const scale = Math.min(cw / iw, ch / ih)
      const dw = Math.round(iw * scale)
      const dh = Math.round(ih * scale)
      const dx = Math.round((cw - dw) / 2)
      const dy = Math.round((ch - dh) / 2)
      context.drawImage(frame, dx, dy, dw, dh)
    } finally {
      frame.close()
    }
  }

  private closeVideoDecoder() {
    if (this.videoDecoder && this.videoDecoder.state !== 'closed') {
      this.videoDecoder.close()
    }
    this.videoDecoder = undefined
    this.videoDecoderConfigKey = undefined
  }

  private detectImageMimeType(payload: Uint8Array): string | undefined {
    if (
      payload.byteLength >= 8 &&
      payload[0] === 0x89 &&
      payload[1] === 0x50 &&
      payload[2] === 0x4e &&
      payload[3] === 0x47 &&
      payload[4] === 0x0d &&
      payload[5] === 0x0a &&
      payload[6] === 0x1a &&
      payload[7] === 0x0a
    ) {
      return 'image/png'
    }

    const header = decodeUtf8(payload.subarray(0, Math.min(payload.byteLength, 64))).trimStart()
    if (header.startsWith('<svg') || header.startsWith('<?xml')) {
      return 'image/svg+xml'
    }

    return undefined
  }
}

function decodeOptionalBase64(value: string | undefined): Uint8Array | undefined {
  if (!value) {
    return undefined
  }

  const decoded = atob(value)
  const bytes = new Uint8Array(decoded.length)
  for (let index = 0; index < decoded.length; index += 1) {
    bytes[index] = decoded.charCodeAt(index)
  }
  return bytes
}
