import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
}

// ABIs: by default one APK holds arm64-v8a (devices) and x86_64 (emulators). F-Droid builds one
// APK per ABI with -PsquibAbi=<abi>; each gets the release's version code times ten plus the
// ABI's number below, so the per-ABI codes are distinct and above the universal one.
val abiNumbers = mapOf("armeabi-v7a" to 1, "arm64-v8a" to 2, "x86_64" to 3)
val squibAbi = findProperty("squibAbi") as String?
val buildAbis = squibAbi?.let { listOf(it) } ?: listOf("arm64-v8a", "x86_64")
require(squibAbi == null || squibAbi in listOf("arm64-v8a", "x86_64")) { "unsupported squibAbi $squibAbi" }

// Rust core: cross-compiled with cargo-ndk and bound with UniFFI before every build.
// Set -PsquibSkipRust=true to reuse previously built libraries (e.g. offline iteration).
val repoRoot = rootProject.projectDir.parentFile.parentFile
val rustProfile = (findProperty("squibRustProfile") as String?) ?: "release"
val buildRustCore = tasks.register<Exec>("buildRustCore") {
    group = "build"
    description = "Build libsquib_mobile.so for Android ABIs and generate Kotlin bindings"
    onlyIf { (findProperty("squibSkipRust") as String?) != "true" }
    workingDir = repoRoot
    commandLine(listOf("bash", "scripts/build-android-core.sh", rustProfile) + buildAbis)
    inputs.dir(File(repoRoot, "crates"))
    inputs.file(File(repoRoot, "Cargo.lock"))
    inputs.file(File(repoRoot, "scripts/build-android-core.sh"))
    outputs.dir("src/main/jniLibs")
    outputs.dir("src/main/generated/uniffi")
}
// The About screen shows the full licence offline; it is copied from the repository root so
// there is one copy to keep current.
val aboutAssets = layout.buildDirectory.dir("generated/aboutAssets")
val copyLicense = tasks.register<Copy>("copyLicense") {
    from(File(repoRoot, "LICENSE")) { rename { "LICENSE.txt" } }
    into(aboutAssets)
}
tasks.named("preBuild") { dependsOn(buildRustCore, copyLicense) }

// Version policy (ADR-058): versionName is MAJOR.MINOR.PATCH and versionCode is
// MAJOR*10000 + MINOR*100 + PATCH, so each release has a unique, increasing code that does not
// depend on the build machine, the clock or git history (F-Droid builds from source and must get
// the same code). Both are written as literals because F-Droid's update checker reads them from
// this file with a regular expression (UpdateCheckData in the F-Droid recipe); the check below
// keeps the code in step with the name.
val squibVersion = "0.1.2"
val squibVersionCode = 102
squibVersion.split(".").map(String::toInt).let { (major, minor, patch) ->
    require(minor < 100 && patch < 100) { "minor and patch must be below 100" }
    require(squibVersionCode == major * 10_000 + minor * 100 + patch) {
        "squibVersionCode $squibVersionCode does not match squibVersion $squibVersion"
    }
}

// Release signing comes only from outside the repository: a git-ignored
// apps/android/keystore.properties (storeFile, storePassword, keyAlias, keyPassword) or the
// SQUIB_KEYSTORE, SQUIB_KEYSTORE_PASSWORD, SQUIB_KEY_ALIAS and SQUIB_KEY_PASSWORD environment
// variables. With neither, the release APK is built unsigned (as in CI).
val keystoreProps = Properties().apply {
    rootProject.file("keystore.properties").takeIf { it.isFile }?.inputStream()?.use { load(it) }
}
fun signingValue(key: String, env: String): String? = keystoreProps.getProperty(key) ?: System.getenv(env)
val releaseStoreFile = signingValue("storeFile", "SQUIB_KEYSTORE")
// Local testing only: sign the minified release with the debug key so it can be installed on
// an emulator. Never used for a build meant for anyone else.
val signReleaseWithDebugKey = (findProperty("squibSignReleaseWithDebugKey") as String?) == "true"

android {
    // Provisional identifiers: the reverse-domain application ID is an open owner
    // decision (docs/squib/README). See docs/squib/decisions/ADR-014.
    namespace = "dev.digitalgrease.squib"
    compileSdk = 37
    // Lets AGP strip debug symbols from the Rust library in the APK (unstripped copies stay
    // under build/intermediates for symbolication).
    ndkVersion = "28.2.13676358"
    // Instrumented tests normally run against debug; -PsquibTestBuildType=release runs the
    // same suite against the minified build to check the keep rules (ADR-058).
    testBuildType = (findProperty("squibTestBuildType") as String?) ?: "debug"

    defaultConfig {
        applicationId = "dev.digitalgrease.squib"
        minSdk = 26
        targetSdk = 36
        versionCode = squibAbi?.let { squibVersionCode * 10 + abiNumbers.getValue(it) } ?: squibVersionCode
        versionName = squibVersion
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // Long opt-in tests (the 30-minute capture soak) stay out of the default suite; see
        // CaptureSoakTest for how to run them.
        testInstrumentationRunnerArguments["notAnnotation"] = "dev.digitalgrease.squib.Soak"
        ndk {
            abiFilters += buildAbis
        }
    }

    sourceSets {
        getByName("main").kotlin.directories += "src/main/generated/uniffi"
        getByName("main").assets.directories += "build/generated/aboutAssets"
    }

    signingConfigs {
        if (releaseStoreFile != null) {
            create("release") {
                storeFile = file(releaseStoreFile)
                storePassword = signingValue("storePassword", "SQUIB_KEYSTORE_PASSWORD")
                keyAlias = signingValue("keyAlias", "SQUIB_KEY_ALIAS")
                keyPassword = signingValue("keyPassword", "SQUIB_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            // Building is not publishing: distribution needs separate authorization.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            testProguardFiles("test-proguard-rules.pro")
            if (testBuildType == "release") proguardFiles("proguard-test-target.pro")
            signingConfig = when {
                signReleaseWithDebugKey -> signingConfigs.getByName("debug")
                releaseStoreFile != null -> signingConfigs.getByName("release")
                else -> null
            }
        }
    }

    // F-Droid refuses APKs carrying Google's encrypted dependency-metadata block.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

java {
    toolchain {
        languageVersion = JavaLanguageVersion.of(21)
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
    }
}

dependencies {
    implementation(libs.core.ktx)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.activity.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.coroutines.android)
    implementation("${libs.jna.get()}@aar")
    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit)
    androidTestImplementation(libs.junit.ext)
    androidTestImplementation(libs.test.runner)
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.compose.ui.test.junit4.accessibility)
}
