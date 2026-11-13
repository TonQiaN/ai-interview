// 运行在音频线程里的 AudioWorklet：每 20 毫秒算一次麦克风音量（RMS），发给主线程。
//
// 放在音频线程而不是主线程定时读，是因为页面切到后台时主线程的定时器会被浏览器降频，
// 音频线程不受影响，静音检测不会因此失灵。
//
// 这个文件按原样作为独立脚本加载（不经过打包），所以是纯 JavaScript，不能 import 别的模块。

const FRAME_SECONDS = 0.02;

class LevelProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.frameSize = Math.max(1, Math.round(sampleRate * FRAME_SECONDS));
    this.sumSquares = 0;
    this.count = 0;
  }

  process(inputs) {
    const channel = inputs[0]?.[0];
    // 输入断开（麦克风被拔掉）时 channel 是 undefined，按静音处理，保持帧率不变
    const length = channel ? channel.length : 128;
    for (let i = 0; i < length; i += 1) {
      const sample = channel ? channel[i] : 0;
      this.sumSquares += sample * sample;
      this.count += 1;
      if (this.count >= this.frameSize) {
        this.port.postMessage({
          rms: Math.sqrt(this.sumSquares / this.count),
          frameMs: (this.count / sampleRate) * 1000,
        });
        this.sumSquares = 0;
        this.count = 0;
      }
    }
    return true;
  }
}

registerProcessor("level-processor", LevelProcessor);
