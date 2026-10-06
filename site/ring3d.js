// A titanium smart ring in Three.js, shared by the landing page and the demo film.
// Everything is a function of `t` (seconds) and a few state values, so a video
// renderer can seek any frame and a web page can drive it from scroll or time.
import * as THREE from 'three'
import { RoomEnvironment } from 'three/addons/environments/RoomEnvironment.js'
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js'
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js'
import { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js'
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js'

// Deterministic PRNG (mulberry32): the same particles on every frame and every load.
function rng(seed) {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0
    let x = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x
    return ((x ^ (x >>> 14)) >>> 0) / 4294967296
  }
}

// Sleep-stage palette, shared with the site: awake, REM, core, deep.
export const STAGE_COLORS = [0xff9f0a, 0x64d2ff, 0x2f8cff, 0x5e5ce6]
// World y of each hypnogram lane (awake, REM, core, deep).
export const LANES = [0.95, 0.4, -0.15, -0.7]

function bandProfile() {
  // Half cross-section of the band (x = radius, y = height), rounded corners.
  const rIn = 1.0, rOut = 1.26, h = 0.34, c = 0.085, pts = []
  const arc = (cx, cy, a0, a1) => {
    for (let i = 0; i <= 10; i++) {
      const a = a0 + (a1 - a0) * (i / 10)
      pts.push(new THREE.Vector2(cx + Math.cos(a) * c, cy + Math.sin(a) * c))
    }
  }
  // Outer face bulges slightly, like a Horizon ring.
  arc(rIn + c, -h + c, Math.PI, Math.PI * 1.5)
  arc(rOut - c, -h + c, -Math.PI / 2, 0)
  for (let i = 1; i < 16; i++) {
    const y = -h + c + (2 * (h - c)) * (i / 16)
    pts.push(new THREE.Vector2(rOut + 0.028 * Math.cos((y / (h - c)) * Math.PI / 2), y))
  }
  arc(rOut - c, h - c, 0, Math.PI / 2)
  arc(rIn + c, h - c, Math.PI / 2, Math.PI)
  pts.push(pts[0].clone())
  return pts
}

// One real night from the ring (120 epochs, 03:53-13:21): 1 deep, 2 core, 3 REM, 4 awake.
export const NIGHT = [4,1,1,1,1,2,2,1,1,2,2,2,1,1,1,1,2,2,1,4,2,1,1,1,1,2,2,1,1,2,2,2,1,1,1,1,2,2,1,1,1,1,1,1,2,2,2,1,1,1,2,3,3,3,2,2,2,2,2,2,2,2,2,1,1,2,2,2,2,2,2,4,2,2,2,1,1,2,2,2,2,2,2,2,2,2,4,3,3,3,3,3,2,2,2,2,2,2,2,2,2,2,2,2,3,3,3,2,4,4,2,2,1,1,1,1,1,2,3,3]

export function createRing(canvas, { width, height, pixelRatio = 1, transparent = true, bloom = 0.9, night = NIGHT } = {}) {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: transparent, preserveDrawingBuffer: true })
  renderer.setPixelRatio(pixelRatio)
  renderer.setSize(width, height, false)
  renderer.toneMapping = THREE.ACESFilmicToneMapping
  renderer.toneMappingExposure = 0.85
  renderer.setClearColor(0x000000, 0)

  const scene = new THREE.Scene()
  const pmrem = new THREE.PMREMGenerator(renderer)
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture
  scene.environmentIntensity = 0.55

  const camera = new THREE.PerspectiveCamera(30, width / height, 0.1, 100)
  camera.position.set(0, 0.9, 6.4)

  // Key lights: a cool rim from behind and a warm kicker, both moving with t.
  const rim = new THREE.DirectionalLight(0x9fd8ff, 2.4); scene.add(rim)
  const kick = new THREE.DirectionalLight(0xffe2c4, 1.4); scene.add(kick)
  scene.add(new THREE.AmbientLight(0x223044, 0.4))

  const ring = new THREE.Group(); scene.add(ring)
  const metal = new THREE.MeshPhysicalMaterial({
    color: 0x1d1f23, metalness: 1, roughness: 0.26, clearcoat: 0.55, clearcoatRoughness: 0.1,
    envMapIntensity: 1.0,
  })
  const band = new THREE.Mesh(new THREE.LatheGeometry(bandProfile(), 220), metal)
  ring.add(band)

  // Inner sensor domes: three green LEDs and a red one, on the palm side.
  const leds = []
  const ledGeo = new THREE.SphereGeometry(0.05, 24, 16)
  ;[[-0.32, 0x30ff9a], [0, 0x30ff9a], [0.32, 0x30ff9a], [0.62, 0xff3b52]].forEach(([a, col], i) => {
    const m = new THREE.MeshBasicMaterial({ color: col, transparent: true, opacity: 1 })
    const dome = new THREE.Mesh(ledGeo, m)
    const ang = -Math.PI / 2 + a
    dome.position.set(Math.cos(ang) * 0.985, (i === 3 ? 0.12 : -0.05), Math.sin(ang) * 0.985)
    dome.scale.set(1, 0.7, 1)
    ring.add(dome); leds.push(dome)
  })

  // Data particles: 2,400 points that leave the ring and fall into lanes (the four
  // sleep stages) or flow along a stream. State comes from `flow` and `t` only.
  const N = 2400, rand = rng(7)
  const seeds = new Float32Array(N * 4)
  // Lane of each particle = the stage of the night at its x position (awake on top).
  const lane = { 4: 0, 3: 1, 2: 2, 1: 3 }
  for (let i = 0; i < N; i++) {
    const a = rand()
    seeds[i * 4] = a; seeds[i * 4 + 1] = rand(); seeds[i * 4 + 2] = rand()
    seeds[i * 4 + 3] = lane[night[Math.min(night.length - 1, Math.floor(a * night.length))]]
  }
  const pos = new Float32Array(N * 3), col = new Float32Array(N * 3)
  const pgeo = new THREE.BufferGeometry()
  pgeo.setAttribute('position', new THREE.BufferAttribute(pos, 3))
  pgeo.setAttribute('color', new THREE.BufferAttribute(col, 3))
  const sprite = (() => {
    const c = document.createElement('canvas'); c.width = c.height = 64
    const g = c.getContext('2d'), grd = g.createRadialGradient(32, 32, 0, 32, 32, 32)
    grd.addColorStop(0, 'rgba(255,255,255,1)'); grd.addColorStop(0.35, 'rgba(255,255,255,0.55)'); grd.addColorStop(1, 'rgba(255,255,255,0)')
    g.fillStyle = grd; g.fillRect(0, 0, 64, 64)
    return new THREE.CanvasTexture(c)
  })()
  const pmat = new THREE.PointsMaterial({ size: 0.05, map: sprite, vertexColors: true, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending })
  const points = new THREE.Points(pgeo, pmat); scene.add(points)
  const stageColor = STAGE_COLORS.map(c => new THREE.Color(c))

  let composer = null, bloomPass = null
  if (bloom > 0) {
    composer = new EffectComposer(renderer)
    composer.setPixelRatio(pixelRatio)
    composer.setSize(width, height)
    composer.addPass(new RenderPass(scene, camera))
    bloomPass = new UnrealBloomPass(new THREE.Vector2(width, height), bloom, 0.45, 0.78)
    composer.addPass(bloomPass)
    composer.addPass(new OutputPass())
  }

  const smooth = x => x * x * (3 - 2 * x)
  const clamp01 = x => Math.max(0, Math.min(1, x))

  /**
   * state:
   *  spin      – ring yaw in radians (added to a slow idle turn)
   *  tilt      – ring pitch in radians
   *  zoom      – camera distance multiplier (1 = default)
   *  lift      – vertical offset of the ring
   *  pulse     – 0..1 LED intensity multiplier (a heartbeat can drive it)
   *  flow      – 0 = particles orbit the ring, 1 = particles laid out as a hypnogram
   *  stream    – 0..1 how far particles have streamed toward `target`
   *  target    – [x,y,z] where the stream goes (e.g. a phone)
   *  particles – 0..1 opacity of the particle field
   *  hue       – 0..1 metal tint from stealth black (0) to brushed titanium (1)
 *  x         – horizontal offset of the ring
 *  idle      – 0..1 how much of the slow idle turn is applied
   */
  function renderAt(t, s = {}) {
    const spin = s.spin ?? 0, tilt = s.tilt ?? 0.42, zoom = s.zoom ?? 1, lift = s.lift ?? 0, idle = s.idle ?? 1
    const pulse = s.pulse ?? 0.6, flow = s.flow ?? 0, stream = s.stream ?? 0, pa = s.particles ?? 1
    const target = s.target ?? [2.6, -0.2, 0]
    metal.color.setRGB(0.11 + 0.6 * (s.hue ?? 0), 0.12 + 0.6 * (s.hue ?? 0), 0.135 + 0.6 * (s.hue ?? 0))

    ring.rotation.set(tilt + Math.sin(t * 0.4) * 0.05 * idle, spin + t * 0.35 * idle, Math.sin(t * 0.3) * 0.06 * idle)
    ring.position.set(s.x ?? 0, lift + Math.sin(t * 0.8) * 0.03 * idle, 0)
    camera.position.set(Math.sin(t * 0.12) * 0.35 * idle, 0.9 * zoom, 6.4 * zoom)
    camera.lookAt(0, lift * 0.5, 0)
    rim.position.set(Math.cos(t * 0.6) * 5, 3, Math.sin(t * 0.6) * 5 - 2)
    kick.position.set(-4, -1 + Math.sin(t * 0.5), 3)
    leds.forEach((l, i) => { l.material.opacity = clamp01(pulse * (i === 3 ? 0.8 : 1)); l.scale.setScalar(0.6 + 0.6 * pulse); l.scale.y *= 0.7 })

    for (let i = 0; i < N; i++) {
      const a = seeds[i * 4], b = seeds[i * 4 + 1], c = seeds[i * 4 + 2], stage = seeds[i * 4 + 3]
      // Orbit: a thin halo around the band.
      const ang = a * Math.PI * 2 + t * (0.25 + b * 0.4)
      const r = 1.55 + b * 0.9 + Math.sin(t * 0.7 + c * 9) * 0.05
      let x = Math.cos(ang) * r, y = (c - 0.5) * 0.5 + Math.sin(ang * 3 + t) * 0.06, z = Math.sin(ang) * r
      // Hypnogram: x spreads over the night, y is the stage lane (awake on top).
      const hx = (a - 0.5) * 7.2, hy = LANES[stage] + (b - 0.5) * 0.12, hz = (c - 0.5) * 0.25
      const f = smooth(clamp01(flow * 1.25 - a * 0.25))
      x += (hx - x) * f; y += (hy - y) * f; z += (hz - z) * f
      // Stream toward the target, staggered by seed, with a gentle arc.
      const st = smooth(clamp01(stream * 1.6 - b * 0.6))
      if (st > 0) {
        const arc = Math.sin(st * Math.PI) * (0.9 + c)
        x += (target[0] - x) * st; y += (target[1] - y) * st + arc * 0.35; z += (target[2] - z) * st
      }
      pos[i * 3] = x; pos[i * 3 + 1] = y; pos[i * 3 + 2] = z
      const sc = stageColor[stage], fade = pa * (1 - st * 0.85)
      col[i * 3] = sc.r * fade; col[i * 3 + 1] = sc.g * fade; col[i * 3 + 2] = sc.b * fade
    }
    pgeo.attributes.position.needsUpdate = true
    pgeo.attributes.color.needsUpdate = true
    pmat.size = 0.045 + flow * 0.02

    if (composer) { if (s.bloom != null) bloomPass.strength = s.bloom; composer.render() }
    else renderer.render(scene, camera)
  }

  function resize(w, h, pr = pixelRatio) {
    renderer.setPixelRatio(pr); renderer.setSize(w, h, false)
    camera.aspect = w / h; camera.updateProjectionMatrix()
    if (composer) { composer.setPixelRatio(pr); composer.setSize(w, h) }
  }

  // Screen position (CSS pixels of the canvas) of a world point, after the last render.
  function project(x, y, z) {
    const v = new THREE.Vector3(x, y, z).project(camera)
    const r = renderer.domElement.getBoundingClientRect()
    return { x: (v.x + 1) / 2 * r.width, y: (1 - v.y) / 2 * r.height }
  }

  return { renderAt, resize, project, renderer }
}
