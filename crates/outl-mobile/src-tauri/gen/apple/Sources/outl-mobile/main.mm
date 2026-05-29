#include "bindings/bindings.h"

#import <UIKit/UIKit.h>
#import <WebKit/WebKit.h>
#import <BackgroundTasks/BackgroundTasks.h>
#import <objc/runtime.h>

// ---------------------------------------------------------------------------
// Native keyboard accessory toolbar.
//
// Replaces the iOS form input accessory bar (`↑ ↓ ✓`) with our own
// UIView. Buttons are real UIKit, so they live *inside* the keyboard
// frame, animate together with the keyboard, and look pixel-native.
//
// Each tap calls `window.__outlToolbar(action)` in the WebView via
// `evaluateJavaScript`. The Solid frontend registers that handler in
// `Journal.tsx` and dispatches to the same Tauri commands the old
// HTML toolbar used.
// ---------------------------------------------------------------------------

@interface OutlToolbarView : UIView
@property (nonatomic, weak) WKWebView *webView;
@end

@implementation OutlToolbarView

- (instancetype)init {
    self = [super initWithFrame:CGRectMake(0, 0, 0, 46)];
    if (self) {
        self.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *trait) {
            if (trait.userInterfaceStyle == UIUserInterfaceStyleDark) {
                return [UIColor colorWithWhite:0.11 alpha:1.0];
            }
            return [UIColor colorWithWhite:0.97 alpha:1.0];
        }];
        UIView *topLine = [[UIView alloc] init];
        topLine.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *trait) {
            if (trait.userInterfaceStyle == UIUserInterfaceStyleDark) {
                return [UIColor colorWithWhite:1.0 alpha:0.10];
            }
            return [UIColor colorWithWhite:0.0 alpha:0.10];
        }];
        topLine.translatesAutoresizingMaskIntoConstraints = NO;
        [self addSubview:topLine];
        [NSLayoutConstraint activateConstraints:@[
            [topLine.leadingAnchor constraintEqualToAnchor:self.leadingAnchor],
            [topLine.trailingAnchor constraintEqualToAnchor:self.trailingAnchor],
            [topLine.topAnchor constraintEqualToAnchor:self.topAnchor],
            [topLine.heightAnchor constraintEqualToConstant:0.5]
        ]];
        [self setupButtons];
        self.autoresizingMask = UIViewAutoresizingFlexibleWidth;
    }
    return self;
}

- (CGSize)intrinsicContentSize {
    return CGSizeMake(UIViewNoIntrinsicMetric, 46);
}

- (void)setupButtons {
    UIScrollView *scroll = [[UIScrollView alloc] init];
    scroll.translatesAutoresizingMaskIntoConstraints = NO;
    scroll.showsHorizontalScrollIndicator = NO;
    scroll.showsVerticalScrollIndicator = NO;
    scroll.alwaysBounceHorizontal = NO;
    scroll.alwaysBounceVertical = NO;
    scroll.bounces = NO;
    scroll.scrollsToTop = NO;
    [self addSubview:scroll];

    UIStackView *stack = [[UIStackView alloc] init];
    stack.axis = UILayoutConstraintAxisHorizontal;
    stack.alignment = UIStackViewAlignmentCenter;
    stack.spacing = 2;
    stack.translatesAutoresizingMaskIntoConstraints = NO;
    [scroll addSubview:stack];

    // Tone vocabulary:
    //   "normal"      — SF Symbol, labelColor tint
    //   "destructive" — SF Symbol, systemRed tint
    //   "text"        — text label (used for `[[`, `((`, `#` which
    //                   don't have a clean SF Symbol equivalent)
    //   "divider"     — vertical hairline
    // "+" lives in the first slot: creating a new line is the most
    // frequent action while editing, so it's worth one tap from the
    // user's resting thumb position on the toolbar edge.
    NSArray<NSArray<NSString *> *> *items = @[
        @[@"plus",                                       @"newLine",     @"normal"],
        @[@"|",                                          @"",            @"divider"],
        @[@"decrease.indent",                            @"outdent",     @"normal"],
        @[@"increase.indent",                            @"indent",      @"normal"],
        @[@"arrow.up",                                   @"moveUp",      @"normal"],
        @[@"arrow.down",                                 @"moveDown",    @"normal"],
        @[@"|",                                          @"",            @"divider"],
        @[@"bold",                                       @"bold",        @"normal"],
        @[@"italic",                                     @"italic",      @"normal"],
        @[@"chevron.left.forwardslash.chevron.right",    @"code",        @"normal"],
        @[@"|",                                          @"",            @"divider"],
        @[@"[[",                                         @"insertRef",   @"text"],
        @[@"((",                                         @"insertBlock", @"text"],
        @[@"#",                                          @"insertHash",  @"text"],
        @[@"|",                                          @"",            @"divider"],
        @[@"checkmark.circle",                           @"todo",        @"normal"],
        @[@"trash",                                      @"delete",      @"destructive"],
    ];
    for (NSArray<NSString *> *item in items) {
        NSString *tone = item[2];
        if ([tone isEqualToString:@"divider"]) {
            [stack addArrangedSubview:[self makeDivider]];
            continue;
        }
        UIButton *btn = [tone isEqualToString:@"text"]
            ? [self buttonWithText:item[0] action:item[1]]
            : [self buttonWithSymbol:item[0] action:item[1] tone:tone];
        [stack addArrangedSubview:btn];
    }

    UIView *spacer = [[UIView alloc] init];
    spacer.translatesAutoresizingMaskIntoConstraints = NO;
    [spacer setContentHuggingPriority:1 forAxis:UILayoutConstraintAxisHorizontal];
    [stack addArrangedSubview:spacer];

    UIButton *done = [UIButton buttonWithType:UIButtonTypeSystem];
    [done setTitle:@"Done" forState:UIControlStateNormal];
    [done setTitleColor:[UIColor systemBlueColor] forState:UIControlStateNormal];
    [done.titleLabel setFont:[UIFont systemFontOfSize:16 weight:UIFontWeightSemibold]];
    [done addTarget:self action:@selector(doneTapped) forControlEvents:UIControlEventTouchUpInside];
    done.contentEdgeInsets = UIEdgeInsetsMake(0, 12, 0, 12);
    [stack addArrangedSubview:done];

    [NSLayoutConstraint activateConstraints:@[
        [scroll.leadingAnchor constraintEqualToAnchor:self.leadingAnchor],
        [scroll.trailingAnchor constraintEqualToAnchor:self.trailingAnchor],
        [scroll.topAnchor constraintEqualToAnchor:self.topAnchor constant:0.5],
        [scroll.bottomAnchor constraintEqualToAnchor:self.bottomAnchor],

        [stack.leadingAnchor constraintEqualToAnchor:scroll.leadingAnchor constant:6],
        [stack.trailingAnchor constraintEqualToAnchor:scroll.trailingAnchor constant:-6],
        [stack.topAnchor constraintEqualToAnchor:scroll.topAnchor],
        [stack.bottomAnchor constraintEqualToAnchor:scroll.bottomAnchor],
        [stack.heightAnchor constraintEqualToAnchor:scroll.heightAnchor]
    ]];
}

- (UIButton *)buttonWithSymbol:(NSString *)symbol action:(NSString *)action tone:(NSString *)tone {
    UIButton *btn = [UIButton buttonWithType:UIButtonTypeSystem];
    UIImageSymbolConfiguration *cfg =
        [UIImageSymbolConfiguration configurationWithPointSize:18 weight:UIImageSymbolWeightRegular];
    UIImage *img = [[UIImage systemImageNamed:symbol withConfiguration:cfg]
                    imageWithRenderingMode:UIImageRenderingModeAlwaysTemplate];
    [btn setImage:img forState:UIControlStateNormal];
    btn.tintColor = [tone isEqualToString:@"destructive"]
        ? [UIColor systemRedColor]
        : [UIColor labelColor];
    btn.translatesAutoresizingMaskIntoConstraints = NO;
    [NSLayoutConstraint activateConstraints:@[
        [btn.widthAnchor constraintEqualToConstant:42],
        [btn.heightAnchor constraintEqualToConstant:38]
    ]];
    objc_setAssociatedObject(
        btn, "outlAction", action, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [btn addTarget:self
            action:@selector(buttonTapped:)
  forControlEvents:UIControlEventTouchUpInside];
    return btn;
}

- (UIButton *)buttonWithText:(NSString *)title action:(NSString *)action {
    UIButton *btn = [UIButton buttonWithType:UIButtonTypeSystem];
    [btn setTitle:title forState:UIControlStateNormal];
    [btn setTitleColor:[UIColor labelColor] forState:UIControlStateNormal];
    [btn.titleLabel setFont:[UIFont monospacedSystemFontOfSize:16 weight:UIFontWeightMedium]];
    btn.translatesAutoresizingMaskIntoConstraints = NO;
    [NSLayoutConstraint activateConstraints:@[
        [btn.widthAnchor constraintEqualToConstant:42],
        [btn.heightAnchor constraintEqualToConstant:38]
    ]];
    objc_setAssociatedObject(
        btn, "outlAction", action, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [btn addTarget:self
            action:@selector(buttonTapped:)
  forControlEvents:UIControlEventTouchUpInside];
    return btn;
}

- (UIView *)makeDivider {
    UIView *line = [[UIView alloc] init];
    line.translatesAutoresizingMaskIntoConstraints = NO;
    line.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *trait) {
        if (trait.userInterfaceStyle == UIUserInterfaceStyleDark) {
            return [UIColor colorWithWhite:1.0 alpha:0.18];
        }
        return [UIColor colorWithWhite:0.0 alpha:0.18];
    }];
    [NSLayoutConstraint activateConstraints:@[
        [line.widthAnchor constraintEqualToConstant:0.5],
        [line.heightAnchor constraintEqualToConstant:22]
    ]];
    return line;
}

- (void)buttonTapped:(UIButton *)sender {
    NSString *action = objc_getAssociatedObject(sender, "outlAction");
    if (!action) return;
    [self invokeAction:action];
}

- (void)doneTapped {
    [self invokeAction:@"done"];
}

- (void)invokeAction:(NSString *)action {
    WKWebView *web = [self resolveWebView];
    if (!web) return;
    NSString *escaped = [action stringByReplacingOccurrencesOfString:@"'" withString:@"\\'"];
    NSString *js =
        [NSString stringWithFormat:@"window.__outlToolbar && window.__outlToolbar('%@')", escaped];
    [web evaluateJavaScript:js completionHandler:nil];
}

- (WKWebView *)resolveWebView {
    if (self.webView) return self.webView;
    UIScene *scene = [UIApplication sharedApplication].connectedScenes.anyObject;
    UIWindow *window = nil;
    if ([scene isKindOfClass:[UIWindowScene class]]) {
        UIWindowScene *windowScene = (UIWindowScene *)scene;
        if (@available(iOS 15.0, *)) {
            window = windowScene.keyWindow;
        }
        if (!window) {
            window = windowScene.windows.firstObject;
        }
    }
    if (!window) {
        window = UIApplication.sharedApplication.windows.firstObject;
    }
    WKWebView *web = [OutlToolbarView findWebViewIn:window];
    self.webView = web;
    return web;
}

+ (WKWebView *)findWebViewIn:(UIView *)view {
    if (!view) return nil;
    if ([view isKindOfClass:[WKWebView class]]) return (WKWebView *)view;
    for (UIView *sub in view.subviews) {
        WKWebView *found = [self findWebViewIn:sub];
        if (found) return found;
    }
    return nil;
}

@end

// ---------------------------------------------------------------------------
// Native ref suggester (chips floating above the keyboard)
//
// When the user is typing inside `[[…]]` the JS side writes a payload
// to `window.__outlSuggesterState`. The overlay below polls that
// value while the keyboard is up, repositions itself above the
// keyboard via `UIKeyboardWillShowNotification`, and renders the
// matching pages as horizontally-scrolling chips. Tap → we
// `evaluateJavaScript window.__outlSuggesterPicked(slug, kind)` and
// the JS layer rewrites the textarea synchronously so the keyboard
// stays up.
//
// We deliberately keep this *outside* the inputAccessoryView. A
// previous version embedded the chip strip inside the toolbar and
// the AutoLayout intrinsic-content-size juggling collapsed the
// whole accessory to 0pt under some keyboard transitions. An
// overlay in the key window is bulletproof.
// ---------------------------------------------------------------------------

@interface OutlSuggestView : UIView
@property (nonatomic, weak) WKWebView *webView;
@property (nonatomic, strong) UIStackView *chipStack;
@property (nonatomic, strong) UIScrollView *scroll;
@property (nonatomic, assign) BOOL visible;
- (void)showItems:(NSArray<NSDictionary *> *)items;
- (void)hide;
@end

@implementation OutlSuggestView

- (instancetype)init {
    self = [super initWithFrame:CGRectMake(0, 0, 0, 0)];
    if (self) {
        // Match the formatting toolbar's background exactly so the
        // strip reads as one continuous slab when both are visible.
        self.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *trait) {
            if (trait.userInterfaceStyle == UIUserInterfaceStyleDark) {
                return [UIColor colorWithWhite:0.11 alpha:1.0];
            }
            return [UIColor colorWithWhite:0.97 alpha:1.0];
        }];
        // Top hairline mirrors the toolbar's own top line so the
        // floating strip looks anchored to the keyboard stack.
        UIView *topLine = [[UIView alloc] init];
        topLine.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *trait) {
            if (trait.userInterfaceStyle == UIUserInterfaceStyleDark) {
                return [UIColor colorWithWhite:1.0 alpha:0.10];
            }
            return [UIColor colorWithWhite:0.0 alpha:0.10];
        }];
        topLine.translatesAutoresizingMaskIntoConstraints = NO;
        [self addSubview:topLine];
        [NSLayoutConstraint activateConstraints:@[
            [topLine.leadingAnchor constraintEqualToAnchor:self.leadingAnchor],
            [topLine.trailingAnchor constraintEqualToAnchor:self.trailingAnchor],
            [topLine.topAnchor constraintEqualToAnchor:self.topAnchor],
            [topLine.heightAnchor constraintEqualToConstant:0.5]
        ]];

        self.scroll = [[UIScrollView alloc] init];
        self.scroll.translatesAutoresizingMaskIntoConstraints = NO;
        self.scroll.showsHorizontalScrollIndicator = NO;
        self.scroll.alwaysBounceHorizontal = YES;
        self.scroll.contentInset = UIEdgeInsetsZero;
        [self addSubview:self.scroll];

        self.chipStack = [[UIStackView alloc] init];
        self.chipStack.axis = UILayoutConstraintAxisHorizontal;
        self.chipStack.alignment = UIStackViewAlignmentCenter;
        self.chipStack.spacing = 4;
        self.chipStack.translatesAutoresizingMaskIntoConstraints = NO;
        [self.scroll addSubview:self.chipStack];

        [NSLayoutConstraint activateConstraints:@[
            [self.scroll.leadingAnchor constraintEqualToAnchor:self.leadingAnchor],
            [self.scroll.trailingAnchor constraintEqualToAnchor:self.trailingAnchor],
            [self.scroll.topAnchor constraintEqualToAnchor:self.topAnchor],
            [self.scroll.bottomAnchor constraintEqualToAnchor:self.bottomAnchor],
            [self.chipStack.leadingAnchor constraintEqualToAnchor:self.scroll.leadingAnchor constant:8],
            [self.chipStack.trailingAnchor constraintEqualToAnchor:self.scroll.trailingAnchor constant:-8],
            [self.chipStack.topAnchor constraintEqualToAnchor:self.scroll.topAnchor],
            [self.chipStack.bottomAnchor constraintEqualToAnchor:self.scroll.bottomAnchor],
            [self.chipStack.heightAnchor constraintEqualToAnchor:self.scroll.heightAnchor]
        ]];
        self.visible = NO;
    }
    return self;
}

- (CGSize)intrinsicContentSize {
    // 36pt is the iOS QuickType strip height — visually familiar.
    return CGSizeMake(UIViewNoIntrinsicMetric, self.visible ? 36 : 0);
}

- (void)showItems:(NSArray<NSDictionary *> *)items {
    // Clear current chips.
    for (UIView *sub in [self.chipStack.arrangedSubviews copy]) {
        [self.chipStack removeArrangedSubview:sub];
        [sub removeFromSuperview];
    }
    // QuickType-style: insert a thin vertical divider between chips.
    for (NSUInteger i = 0; i < items.count; i++) {
        if (i > 0) {
            [self.chipStack addArrangedSubview:[self makeDivider]];
        }
        NSDictionary *item = items[i];
        NSString *title = item[@"title"] ?: @"";
        NSString *slug = item[@"slug"] ?: @"";
        NSString *kind = item[@"kind"] ?: @"page";
        UIButton *chip = [self chipWithTitle:title slug:slug kind:kind];
        [self.chipStack addArrangedSubview:chip];
    }
    BOOL wantVisible = items.count > 0;
    if (wantVisible == self.visible) return;
    self.visible = wantVisible;
    [self invalidateIntrinsicContentSize];
    [self.superview invalidateIntrinsicContentSize];
    [self.superview setNeedsLayout];
}

- (UIView *)makeDivider {
    UIView *line = [[UIView alloc] init];
    line.translatesAutoresizingMaskIntoConstraints = NO;
    line.backgroundColor = [UIColor colorWithDynamicProvider:^UIColor *(UITraitCollection *trait) {
        if (trait.userInterfaceStyle == UIUserInterfaceStyleDark) {
            return [UIColor colorWithWhite:1.0 alpha:0.15];
        }
        return [UIColor colorWithWhite:0.0 alpha:0.15];
    }];
    [NSLayoutConstraint activateConstraints:@[
        [line.widthAnchor constraintEqualToConstant:0.5],
        [line.heightAnchor constraintEqualToConstant:18]
    ]];
    return line;
}

- (void)hide {
    [self showItems:@[]];
}

- (UIButton *)chipWithTitle:(NSString *)title slug:(NSString *)slug kind:(NSString *)kind {
    UIButton *btn = [UIButton buttonWithType:UIButtonTypeSystem];
    [btn setTitle:title forState:UIControlStateNormal];
    // Same family/weight Apple uses on QuickType so the strip looks
    // native and the user reads it without re-tuning.
    [btn.titleLabel setFont:[UIFont systemFontOfSize:15 weight:UIFontWeightRegular]];
    [btn setTitleColor:[UIColor labelColor] forState:UIControlStateNormal];
    btn.contentEdgeInsets = UIEdgeInsetsMake(0, 10, 0, 10);
    btn.translatesAutoresizingMaskIntoConstraints = NO;
    [NSLayoutConstraint activateConstraints:@[
        [btn.heightAnchor constraintEqualToConstant:30]
    ]];
    objc_setAssociatedObject(btn, "outlPickSlug", slug, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    objc_setAssociatedObject(btn, "outlPickKind", kind, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [btn addTarget:self
            action:@selector(chipTapped:)
  forControlEvents:UIControlEventTouchUpInside];
    return btn;
}

- (void)chipTapped:(UIButton *)sender {
    NSString *slug = objc_getAssociatedObject(sender, "outlPickSlug") ?: @"";
    NSString *kind = objc_getAssociatedObject(sender, "outlPickKind") ?: @"page";
    NSString *escapedSlug = [slug stringByReplacingOccurrencesOfString:@"'" withString:@"\\'"];
    NSString *escapedKind = [kind stringByReplacingOccurrencesOfString:@"'" withString:@"\\'"];
    NSString *js = [NSString stringWithFormat:
        @"window.__outlSuggesterPicked && window.__outlSuggesterPicked('%@', '%@')",
        escapedSlug, escapedKind];
    [self.webView evaluateJavaScript:js completionHandler:nil];
}

@end

// ---------------------------------------------------------------------------
// Suggest overlay
//
// Floating UIView added to the key window. Listens to keyboard
// notifications to position itself flush above the keyboard, polls
// `window.__outlSuggesterState` every 150ms while visible, and
// updates the embedded `OutlSuggestView` accordingly.
// ---------------------------------------------------------------------------

@interface OutlSuggestOverlay : UIView
@property (nonatomic, weak) WKWebView *webView;
@property (nonatomic, strong) OutlSuggestView *suggest;
@property (nonatomic, assign) BOOL polling;
@property (nonatomic, assign) BOOL keyboardVisible;
@property (nonatomic, assign) CGRect keyboardFrame;
@property (nonatomic, copy) NSString *lastStateSignature;
+ (void)installInKeyWindowWithWebView:(WKWebView *)web;
@end

@implementation OutlSuggestOverlay

static OutlSuggestOverlay *_sharedOverlay = nil;

+ (void)installInKeyWindowWithWebView:(WKWebView *)web {
    UIWindow *win = nil;
    for (UIScene *s in [UIApplication sharedApplication].connectedScenes) {
        if ([s isKindOfClass:[UIWindowScene class]]) {
            UIWindowScene *ws = (UIWindowScene *)s;
            if (@available(iOS 15.0, *)) {
                win = ws.keyWindow;
            }
            if (!win) win = ws.windows.firstObject;
            if (win) break;
        }
    }
    if (!win) win = [UIApplication sharedApplication].windows.firstObject;
    if (!win) return;
    if (!_sharedOverlay) {
        _sharedOverlay = [[OutlSuggestOverlay alloc] init];
    }
    _sharedOverlay.webView = web;
    _sharedOverlay.suggest.webView = web;
    if (_sharedOverlay.superview != win) {
        [_sharedOverlay removeFromSuperview];
        [win addSubview:_sharedOverlay];
    }
    [_sharedOverlay startPolling];
    NSLog(@"[outl] suggest overlay installed in key window");
}

- (instancetype)init {
    self = [super initWithFrame:CGRectZero];
    if (self) {
        self.hidden = YES;
        self.userInteractionEnabled = YES;
        self.suggest = [[OutlSuggestView alloc] init];
        self.suggest.translatesAutoresizingMaskIntoConstraints = NO;
        [self addSubview:self.suggest];
        [NSLayoutConstraint activateConstraints:@[
            [self.suggest.leadingAnchor constraintEqualToAnchor:self.leadingAnchor],
            [self.suggest.trailingAnchor constraintEqualToAnchor:self.trailingAnchor],
            [self.suggest.topAnchor constraintEqualToAnchor:self.topAnchor],
            [self.suggest.bottomAnchor constraintEqualToAnchor:self.bottomAnchor]
        ]];
        [[NSNotificationCenter defaultCenter] addObserver:self
            selector:@selector(keyboardWillShow:)
                name:UIKeyboardWillShowNotification
              object:nil];
        [[NSNotificationCenter defaultCenter] addObserver:self
            selector:@selector(keyboardWillChange:)
                name:UIKeyboardWillChangeFrameNotification
              object:nil];
        [[NSNotificationCenter defaultCenter] addObserver:self
            selector:@selector(keyboardWillHide:)
                name:UIKeyboardWillHideNotification
              object:nil];
    }
    return self;
}

- (void)dealloc {
    [[NSNotificationCenter defaultCenter] removeObserver:self];
}

- (void)keyboardWillShow:(NSNotification *)note {
    self.keyboardVisible = YES;
    self.keyboardFrame = [note.userInfo[UIKeyboardFrameEndUserInfoKey] CGRectValue];
    [self layoutAboveKeyboard];
}

- (void)keyboardWillChange:(NSNotification *)note {
    if (!self.keyboardVisible) return;
    self.keyboardFrame = [note.userInfo[UIKeyboardFrameEndUserInfoKey] CGRectValue];
    [self layoutAboveKeyboard];
}

- (void)keyboardWillHide:(NSNotification *)note {
    self.keyboardVisible = NO;
    [self.suggest hide];
    self.hidden = YES;
}

- (void)layoutAboveKeyboard {
    UIWindow *win = (UIWindow *)self.superview;
    if (!win) return;
    // `UIKeyboardFrameEndUserInfoKey` already accounts for our
    // `inputAccessoryView` (the formatting toolbar): `kbTop` is the
    // top edge of that combined unit. We just need to sit flush
    // above it.
    CGFloat kbTop = self.keyboardFrame.origin.y;
    CGFloat overlayHeight = self.suggest.intrinsicContentSize.height;
    self.frame = CGRectMake(0,
                            kbTop - overlayHeight,
                            win.bounds.size.width,
                            overlayHeight);
    self.hidden = (overlayHeight == 0);
}

- (void)startPolling {
    if (self.polling) return;
    self.polling = YES;
    self.lastStateSignature = nil;
    [self pollOnce];
}

- (void)pollOnce {
    if (!self.polling) return;
    WKWebView *web = self.webView;
    if (!web) {
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.3 * NSEC_PER_SEC)),
                       dispatch_get_main_queue(),
                       ^{ [self pollOnce]; });
        return;
    }
    NSString *js = @"JSON.stringify(window.__outlSuggesterState || null)";
    __weak typeof(self) wself = self;
    [web evaluateJavaScript:js completionHandler:^(id result, NSError *err) {
        __strong typeof(wself) sself = wself;
        if (!sself || !sself.polling) return;
        if (!err && [result isKindOfClass:[NSString class]]) {
            [sself applyStateJSON:(NSString *)result];
        }
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.15 * NSEC_PER_SEC)),
                       dispatch_get_main_queue(),
                       ^{ [sself pollOnce]; });
    }];
}

- (void)applyStateJSON:(NSString *)json {
    if ([json isEqualToString:self.lastStateSignature]) return;
    self.lastStateSignature = json;
    if (!json || [json isEqualToString:@"null"]) {
        [self.suggest hide];
        [self layoutAboveKeyboard];
        return;
    }
    NSData *data = [json dataUsingEncoding:NSUTF8StringEncoding];
    NSError *parseErr = nil;
    id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:&parseErr];
    if (parseErr || ![parsed isKindOfClass:[NSDictionary class]]) {
        [self.suggest hide];
        [self layoutAboveKeyboard];
        return;
    }
    NSDictionary *state = parsed;
    NSString *action = state[@"action"];
    if ([action isEqualToString:@"show"]) {
        NSArray *items = state[@"items"];
        if (![items isKindOfClass:[NSArray class]]) items = @[];
        [self.suggest showItems:items];
    } else {
        [self.suggest hide];
    }
    [self layoutAboveKeyboard];
}

@end

// ---------------------------------------------------------------------------
// Swizzle installer
// ---------------------------------------------------------------------------

@interface OutlKeyboardAccessoryDisabler : NSObject
@end

@implementation OutlKeyboardAccessoryDisabler

static OutlToolbarView *_sharedToolbar = nil;

+ (void)load {
    dispatch_async(dispatch_get_main_queue(), ^{
        [OutlKeyboardAccessoryDisabler installSwizzle];
    });
}

+ (void)installSwizzle {
    Class cls = NSClassFromString(@"WKContentView");
    if (!cls) {
        // WKContentView isn't registered until a WKWebView exists.
        // Retry up to ~1s after launch.
        static int retry = 0;
        if (retry >= 10) {
            NSLog(@"[outl] gave up looking for WKContentView");
            return;
        }
        retry += 1;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.1 * NSEC_PER_SEC)),
            dispatch_get_main_queue(),
            ^{ [OutlKeyboardAccessoryDisabler installSwizzle]; }
        );
        return;
    }
    SEL sel = @selector(inputAccessoryView);
    Method m = class_getInstanceMethod(cls, sel);
    if (!m) return;
    IMP newImpl = imp_implementationWithBlock(^UIView *(__unused id _self) {
        if (!_sharedToolbar) {
            _sharedToolbar = [[OutlToolbarView alloc] init];
        }
        return _sharedToolbar;
    });
    method_setImplementation(m, newImpl);
    NSLog(@"[outl] installed native toolbar (with embedded suggester)");

    // Disable iOS's "interactive keyboard dismiss" gesture on the
    // WebView's scroll view. Without this, dragging the outline
    // partially dismisses the keyboard and drags our toolbar along
    // with it.
    [OutlKeyboardAccessoryDisabler disableInteractiveDismiss];

    // Bind the WKWebView reference into the toolbar so it can poll
    // `window.__outlSuggesterState` for suggester updates.
    [OutlKeyboardAccessoryDisabler bindWebView];
}

+ (void)bindWebView {
    UIWindow *win = nil;
    for (UIScene *s in [UIApplication sharedApplication].connectedScenes) {
        if ([s isKindOfClass:[UIWindowScene class]]) {
            win = ((UIWindowScene *)s).windows.firstObject;
            if (win) break;
        }
    }
    if (!win) win = [UIApplication sharedApplication].windows.firstObject;
    WKWebView *web = [OutlToolbarView findWebViewIn:win];
    if (!web) {
        static int retry = 0;
        if (retry >= 20) return;
        retry += 1;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.2 * NSEC_PER_SEC)),
            dispatch_get_main_queue(),
            ^{ [OutlKeyboardAccessoryDisabler bindWebView]; }
        );
        return;
    }
    if (!_sharedToolbar) {
        _sharedToolbar = [[OutlToolbarView alloc] init];
    }
    _sharedToolbar.webView = web;
    [OutlSuggestOverlay installInKeyWindowWithWebView:web];
    NSLog(@"[outl] bound suggester webview (overlay-driven)");
}

+ (void)disableInteractiveDismiss {
    UIWindow *win = nil;
    NSSet<UIScene *> *scenes = [UIApplication sharedApplication].connectedScenes;
    for (UIScene *s in scenes) {
        if ([s isKindOfClass:[UIWindowScene class]]) {
            UIWindowScene *ws = (UIWindowScene *)s;
            win = ws.windows.firstObject;
            if (win) break;
        }
    }
    if (!win) win = [UIApplication sharedApplication].windows.firstObject;
    WKWebView *web = [OutlToolbarView findWebViewIn:win];
    if (!web) {
        // Try again after the WebView mounts.
        static int retry = 0;
        if (retry >= 10) return;
        retry += 1;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.2 * NSEC_PER_SEC)),
            dispatch_get_main_queue(),
            ^{ [OutlKeyboardAccessoryDisabler disableInteractiveDismiss]; }
        );
        return;
    }
    web.scrollView.keyboardDismissMode = UIScrollViewKeyboardDismissModeNone;
    NSLog(@"[outl] disabled interactive keyboard dismiss");
}

@end

// ---------------------------------------------------------------------------
// Background app refresh
//
// We register a BGAppRefreshTask under
// `app.outl.mobile-app.refresh` (declared in Info.plist's
// `BGTaskSchedulerPermittedIdentifiers`). iOS calls the handler
// opportunistically — typically a handful of times per day, only when
// the device has spare battery and network. Most of the heavy lifting
// (pulling sibling devices' `ops-*.jsonl` files) is already done by
// iCloud Documents in the background without us; this hook exists so
// future versions can warm up the workspace or pre-render `.md`
// projections before the user reopens the app.
//
// The handler runs at most ~30 seconds. We schedule the next one and
// finish immediately so we never spin the radio or burn battery.
// ---------------------------------------------------------------------------

static NSString *const kRefreshTaskIdentifier = @"app.outl.mobile-app.refresh";
static const NSTimeInterval kRefreshEvery = 60 * 60; // 1 hour minimum

@interface OutlBackgroundRefresh : NSObject
@end

@implementation OutlBackgroundRefresh

+ (void)load {
    if (@available(iOS 13.0, *)) {
        dispatch_async(dispatch_get_main_queue(), ^{
            [OutlBackgroundRefresh registerTask];
        });
    }
}

+ (void)registerTask API_AVAILABLE(ios(13.0)) {
    BOOL ok = [[BGTaskScheduler sharedScheduler]
        registerForTaskWithIdentifier:kRefreshTaskIdentifier
                           usingQueue:nil
                        launchHandler:^(BGTask *task) {
            // Schedule the next refresh before we finish so iOS keeps
            // calling us. Even if our handler crashed we'd still get
            // future opportunities.
            [OutlBackgroundRefresh scheduleNextRefresh];
            // No-op for now: iCloud Documents already syncs the op
            // log in background. Mark success so iOS budgets us for
            // future runs.
            [task setTaskCompletedWithSuccess:YES];
        }];
    if (ok) {
        NSLog(@"[outl] registered background refresh");
        [OutlBackgroundRefresh scheduleNextRefresh];
    } else {
        NSLog(@"[outl] failed to register background refresh");
    }
}

+ (void)scheduleNextRefresh API_AVAILABLE(ios(13.0)) {
    BGAppRefreshTaskRequest *req =
        [[BGAppRefreshTaskRequest alloc] initWithIdentifier:kRefreshTaskIdentifier];
    req.earliestBeginDate = [NSDate dateWithTimeIntervalSinceNow:kRefreshEvery];
    NSError *err = nil;
    [[BGTaskScheduler sharedScheduler] submitTaskRequest:req error:&err];
    if (err) {
#if TARGET_OS_SIMULATOR
        // BGTaskScheduler isn't backed by the daemon on the
        // simulator, so submit always fails (error 1). The handler
        // registration above is enough for everything else we care
        // about during dev; ignore silently.
        (void)err;
#else
        NSLog(@"[outl] schedule next refresh failed: %@", err.localizedDescription);
#endif
    }
}

@end

// ---------------------------------------------------------------------------
// iCloud real-time watcher
//
// `NSMetadataQuery` is the only public API for being told when iCloud
// documents change. We scope it to `ops-*.jsonl` files anywhere in the
// app's ubiquitous documents and notify the WebView via
// `window.__outlOpsChanged()` so the Solid frontend can reload the
// workspace without the user having to pull-to-refresh.
//
// Updates are debounced (300ms) so a burst of files arriving from
// iCloud only fires one refresh.
// ---------------------------------------------------------------------------

@interface OutlOpsWatcher : NSObject
@property (strong) NSMetadataQuery *query;
@property (assign) BOOL notifyPending;
@end

@implementation OutlOpsWatcher

+ (instancetype)shared {
    static OutlOpsWatcher *inst = nil;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        inst = [[OutlOpsWatcher alloc] init];
    });
    return inst;
}

- (void)start {
    if (self.query) {
        return;
    }
    self.query = [[NSMetadataQuery alloc] init];
    self.query.searchScopes = @[NSMetadataQueryUbiquitousDocumentsScope];
    // v0 contract: `ops-<actor>.jsonl` is the wire format peers append
    // to. iCloud syncs each file independently and each actor only
    // ever writes its own jsonl, so concurrent edits never produce a
    // conflicting file — the CRDT does the merge after we reload.
    // Watching the `.md`s instead would let us react faster but the
    // `.md`s are projections and a peer's `.md` write may be a stale
    // mirror of an op log we haven't seen yet.
    self.query.predicate = [NSPredicate predicateWithFormat:
        @"%K LIKE 'ops-*.jsonl'", NSMetadataItemFSNameKey];

    NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
    [nc addObserver:self
           selector:@selector(onUpdate:)
               name:NSMetadataQueryDidUpdateNotification
             object:self.query];
    [nc addObserver:self
           selector:@selector(onUpdate:)
               name:NSMetadataQueryDidFinishGatheringNotification
             object:self.query];

    [self.query startQuery];
}

- (void)onUpdate:(NSNotification *)note {
    if (self.notifyPending) {
        return;
    }
    self.notifyPending = YES;

    // Snapshot the current query results. We coordinate the download
    // of each one *before* notifying JS so that when JS calls
    // `reload_workspace`, the Rust side's `std::fs::open` reads the
    // actual file contents instead of an evicted iCloud placeholder.
    [self.query disableUpdates];
    NSMutableArray<NSURL *> *urls = [NSMutableArray array];
    for (NSUInteger i = 0; i < self.query.resultCount; i++) {
        NSMetadataItem *item = [self.query resultAtIndex:i];
        NSURL *url = [item valueForAttribute:NSMetadataItemURLKey];
        if (url) {
            [urls addObject:url];
        }
    }
    [self.query enableUpdates];

    dispatch_async(
        dispatch_get_global_queue(QOS_CLASS_UTILITY, 0),
        ^{
            NSFileManager *fm = [NSFileManager defaultManager];
            for (NSURL *url in urls) {
                NSError *startErr = nil;
                [fm startDownloadingUbiquitousItemAtURL:url error:&startErr];

                NSError *coordErr = nil;
                NSFileCoordinator *coord =
                    [[NSFileCoordinator alloc] initWithFilePresenter:nil];
                [coord coordinateReadingItemAtURL:url
                                          options:NSFileCoordinatorReadingForUploading
                                            error:&coordErr
                                       byAccessor:^(NSURL *u) { (void)u; }];
            }

            dispatch_async(dispatch_get_main_queue(), ^{
                self.notifyPending = NO;
                WKWebView *web =
                    [OutlToolbarView findWebViewIn:
                        UIApplication.sharedApplication.windows.firstObject];
                if (!web) {
                    return;
                }
                NSString *js =
                    @"window.__outlOpsChanged && window.__outlOpsChanged()";
                [web evaluateJavaScript:js completionHandler:nil];
            });
        });
}

@end

@interface OutlOpsWatcherBootstrap : NSObject
@end

@implementation OutlOpsWatcherBootstrap

+ (void)load {
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.0 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{ [[OutlOpsWatcher shared] start]; }
    );
}

@end

// ---------------------------------------------------------------------------
// Brand chrome
//
// iOS hands off from `LaunchScreen.storyboard` (painted brand-dark in
// the storyboard) to the Tauri WebView. `WKWebView` defaults to
// `opaque = YES` and a white background — that white flashed before
// the first HTML frame rendered, breaking the visual continuity from
// LaunchScreen → app.
//
// We fix it by walking the window tree as early as possible and:
//   - tinting the UIWindow itself, which covers any gap between view
//     transitions where the window is briefly visible;
//   - making the WKWebView non-opaque with a brand-coloured background
//     so even its very first frame matches the LaunchScreen;
//   - applying the same colour to the WKWebView's scrollView so
//     bounce / overscroll doesn't reveal a white seam.
//
// Brand background is `#0c0814` (RGB 12/8/20). Light mode users see
// brand-dark for the few frames before the JS layer takes over and
// re-paints — same as the LaunchScreen does; consistent, not jarring.
// ---------------------------------------------------------------------------

@interface OutlBrandChrome : NSObject
@end

@implementation OutlBrandChrome

+ (void)load {
    dispatch_async(dispatch_get_main_queue(), ^{
        [OutlBrandChrome apply];
    });
}

+ (UIColor *)brandBackground {
    // #0c0814 — must match the boot splash in index.html and the
    // LaunchScreen.storyboard background colour.
    return [UIColor colorWithRed:12.0/255.0
                            green:8.0/255.0
                             blue:20.0/255.0
                            alpha:1.0];
}

+ (void)apply {
    UIWindow *win = nil;
    for (UIScene *s in [UIApplication sharedApplication].connectedScenes) {
        if ([s isKindOfClass:[UIWindowScene class]]) {
            UIWindowScene *ws = (UIWindowScene *)s;
            if (@available(iOS 15.0, *)) {
                win = ws.keyWindow;
            }
            if (!win) win = ws.windows.firstObject;
            if (win) break;
        }
    }
    if (!win) win = [UIApplication sharedApplication].windows.firstObject;
    if (!win) {
        // Window not in scene graph yet — retry until it is. Cap at
        // ~2s so we never loop forever if something genuinely broke.
        static int retry = 0;
        if (retry >= 20) return;
        retry += 1;
        dispatch_after(
            dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.1 * NSEC_PER_SEC)),
            dispatch_get_main_queue(),
            ^{ [OutlBrandChrome apply]; }
        );
        return;
    }

    UIColor *brand = [OutlBrandChrome brandBackground];
    win.backgroundColor = brand;
    if (win.rootViewController.view) {
        win.rootViewController.view.backgroundColor = brand;
    }

    WKWebView *web = [OutlToolbarView findWebViewIn:win];
    if (web) {
        web.opaque = NO;
        web.backgroundColor = brand;
        web.scrollView.backgroundColor = brand;
        NSLog(@"[outl] brand chrome applied (window + webview)");
        return;
    }
    // WebView not mounted yet — keep polling so we catch its first
    // frame. The window is already tinted so the user sees brand
    // colour throughout this window.
    static int webRetry = 0;
    if (webRetry >= 30) {
        NSLog(@"[outl] brand chrome: webview never mounted, window-only");
        return;
    }
    webRetry += 1;
    dispatch_after(
        dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.1 * NSEC_PER_SEC)),
        dispatch_get_main_queue(),
        ^{ [OutlBrandChrome apply]; }
    );
}

@end

int main(int argc, char * argv[]) {
    ffi::start_app();
    return 0;
}
