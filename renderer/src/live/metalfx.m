// Objective-C owns the MetalFX scaler. wgpu owns the Metal device, textures and command buffer.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#include <math.h>
#if __has_include(<MetalFX/MetalFX.h>)
#import <MetalFX/MetalFX.h>
#endif

void *threert_fx_create(void *device, bool denoise, bool dynamic, uint32_t iw, uint32_t ih, uint32_t ow, uint32_t oh, float *scales) {
    @autoreleasepool {
#if __has_include(<MetalFX/MetalFX.h>)
        if (@available(macOS 13, *)) {
            // Resolve the framework at runtime so the same binary runs on older systems.
            NSBundle *framework = [NSBundle bundleWithPath:@"/System/Library/Frameworks/MetalFX.framework"];
            if (![framework load]) return NULL;
            Class cls = NSClassFromString(denoise ? @"MTLFXTemporalDenoisedScalerDescriptor" : @"MTLFXTemporalScalerDescriptor");
            id<MTLDevice> gpu = (__bridge id<MTLDevice>)device;
            if (!cls || ![cls supportsDevice:gpu]) return NULL;
            // Apple reports output / input (e.g. 1..3). Integer input dimensions can be rounded.
            // Earlier SDKs have no capability query. Limit reuse to this configuration there.
            scales[0] = fminf((float)ow / iw, (float)oh / ih);
            scales[1] = fmaxf((float)ow / iw, (float)oh / ih);
#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 140000
            if ([cls respondsToSelector:@selector(supportedInputContentMaxScaleForDevice:)]) {
                float maxScale = [cls supportedInputContentMaxScaleForDevice:gpu];
                float minScale = [cls supportedInputContentMinScaleForDevice:gpu];
                if (!(minScale > 0 && maxScale >= minScale)) return NULL;
                scales[0] = minScale;
                scales[1] = maxScale;
                if (iw < floorf(ow / maxScale) || ih < floorf(oh / maxScale) ||
                    iw > ceilf(ow / minScale) || ih > ceilf(oh / minScale)) return NULL;
            }
#endif
            id desc = [cls new];
            [desc setInputWidth:dynamic ? ow : iw];
            [desc setInputHeight:dynamic ? oh : ih];
            [desc setOutputWidth:ow];
            [desc setOutputHeight:oh];
            [desc setColorTextureFormat:MTLPixelFormatRGBA16Float];
            [desc setDepthTextureFormat:MTLPixelFormatR32Float];
            [desc setMotionTextureFormat:MTLPixelFormatRG16Float];
            [desc setOutputTextureFormat:MTLPixelFormatRGBA16Float];
            [desc setAutoExposureEnabled:NO];
#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 140400
            if ([desc respondsToSelector:@selector(setReactiveMaskTextureEnabled:)]) {
                [desc setReactiveMaskTextureEnabled:YES];
                [desc setReactiveMaskTextureFormat:MTLPixelFormatR8Unorm];
            }
#endif
            if (denoise) {
#if __has_include(<MetalFX/MTLFXTemporalDenoisedScaler.h>)
                if (@available(macOS 26, *)) {
                    MTLFXTemporalDenoisedScalerDescriptor *d = desc;
                    d.diffuseAlbedoTextureFormat = MTLPixelFormatRGBA8Unorm;
                    d.specularAlbedoTextureFormat = MTLPixelFormatRGBA8Unorm;
                    d.normalTextureFormat = MTLPixelFormatRGBA16Float;
                    d.roughnessTextureFormat = MTLPixelFormatR16Float;
                    d.specularHitDistanceTextureEnabled = YES;
                    d.specularHitDistanceTextureFormat = MTLPixelFormatR16Float;
                    d.denoiseStrengthMaskTextureEnabled = YES;
                    d.denoiseStrengthMaskTextureFormat = MTLPixelFormatR8Unorm;
                    d.transparencyOverlayTextureEnabled = YES;
                    d.transparencyOverlayTextureFormat = MTLPixelFormatRGBA16Float;
                    return (__bridge_retained void *)[d newTemporalDenoisedScalerWithDevice:gpu];
                }
#endif
                return NULL;
            }
            MTLFXTemporalScalerDescriptor *t = desc;
            if (dynamic) {
                t.inputContentPropertiesEnabled = YES;
                t.inputContentMinScale = scales[0];
                t.inputContentMaxScale = scales[1];
            }
            return (__bridge_retained void *)[t newTemporalScalerWithDevice:gpu];
        }
#endif
        return NULL;
    }
}

void threert_fx_release(void *scaler) { CFRelease(scaler); }

// The order is shared with metalfx.rs's input format lists.
void threert_fx_bind(void *scaler, bool denoise, void *const *inputs, void *output) {
#if __has_include(<MetalFX/MetalFX.h>)
    id s = (__bridge id)scaler;
    [s setColorTexture:(__bridge id)inputs[0]];
    [s setDepthTexture:(__bridge id)inputs[1]];
    [s setMotionTexture:(__bridge id)inputs[2]];
#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 140400
    if ([s respondsToSelector:@selector(setReactiveMaskTexture:)]) [s setReactiveMaskTexture:(__bridge id)inputs[3]];
#endif
    [s setOutputTexture:(__bridge id)output];
#if __has_include(<MetalFX/MTLFXTemporalDenoisedScaler.h>)
    if (denoise) {
        id<MTLFXTemporalDenoisedScaler> d = s;
        d.diffuseAlbedoTexture = (__bridge id)inputs[6];
        d.specularAlbedoTexture = (__bridge id)inputs[7];
        d.normalTexture = (__bridge id)inputs[8];
        d.roughnessTexture = (__bridge id)inputs[9];
        d.specularHitDistanceTexture = (__bridge id)inputs[10];
        d.denoiseStrengthMaskTexture = (__bridge id)inputs[5];
        d.transparencyOverlayTexture = (__bridge id)inputs[4];
    }
#endif
#endif
}

void threert_fx_encode(void *scaler, bool denoise, void *command, bool reset, uint32_t iw, uint32_t ih, float jx, float jy,
                       const float *view, const float *projection) {
#if __has_include(<MetalFX/MetalFX.h>)
    @autoreleasepool {
        id s = (__bridge id)scaler;
        [s setDepthReversed:YES];
        [s setJitterOffsetX:jx];
        [s setJitterOffsetY:jy];
        [s setMotionVectorScaleX:1.0f];
        [s setMotionVectorScaleY:1.0f];
        if (denoise) {
#if __has_include(<MetalFX/MTLFXTemporalDenoisedScaler.h>)
            id<MTLFXTemporalDenoisedScaler> d = s;
            simd_float4x4 v, p;
            memcpy(&v, view, sizeof(v));
            memcpy(&p, projection, sizeof(p));
            d.worldToViewMatrix = v;
            d.viewToClipMatrix = p;
            d.shouldResetHistory = reset;
#endif
        } else {
            id<MTLFXTemporalScaler> t = s;
            t.inputContentWidth = iw;
            t.inputContentHeight = ih;
            t.reset = reset;
        }
        id<MTLCommandBuffer> buffer = (__bridge id<MTLCommandBuffer>)command;
        [(id<MTLFXTemporalScaler>)s encodeToCommandBuffer:buffer];
        // wgpu uses unretained Metal command buffers. Keep the scaler (and its private working set)
        // alive even when the next frame switches backend or recreates the surfaces.
        [buffer addCompletedHandler:^(id<MTLCommandBuffer> completed) { (void)completed; (void)[s class]; }];
    }
#endif
}
