from argparse import ArgumentParser
from core.lottery import DarkfiTable
from core.utils import *
from core.darkie import Darkie
from tqdm import tqdm
import os
from core.strategy import random_strategy

AVG_LEN = 10

KP_STEP=5
KP_SEARCH=0.92

KI_STEP=5
KI_SEARCH=1.6

KD_STEP=5
KD_SEARCH=0.1

RUNNING_TIME=5000
NODES = 100

SHIFTING = 0.05

highest_apr = 0.05
highest_acc = 0.2
highest_staked = 0.3
lowest_apr2target_diff = 1

KP='kp'
KI='ki'
KD='kd'

KP_RANGE_MULTIPLIER = 2
KI_RANGE_MULTIPLIER = 2
KD_RANGE_MULTIPLIER = 2

highest_gain = (KP_SEARCH, KI_SEARCH, KD_SEARCH)

parser = ArgumentParser()
parser.add_argument('-p', '--high-precision', action='store_false', default=False)
parser.add_argument('-r', '--randomizenodes', action='store_true', default=True)
parser.add_argument('-t', '--rand-running-time', action='store_true', default=True)
parser.add_argument('-d', '--debug', action='store_false')
args = parser.parse_args()
high_precision = args.high_precision
randomize_nodes = args.randomizenodes
rand_running_time = args.rand_running_time
debug = args.debug

def experiment(controller_type=CONTROLLER_TYPE_DISCRETE, rkp=0, rki=0, rkd=0, distribution=[], hp=True):
    dt = DarkfiTable(ERC20DRK, RUNNING_TIME, controller_type, kp=-0.010399999999938556, ki=-0.0365999996461878, kd=0.03840000000000491, r_kp=rkp, r_ki=rki, r_kd=rkd)
    RND_NODES = random.randint(5, NODES) if randomize_nodes else NODES
    for idx in range(0,RND_NODES):
        darkie = Darkie(distribution[idx], strategy=random_strategy(EPOCH_LENGTH))
        dt.add_darkie(darkie)
    acc, apy, reward, stake_ratio, apr = dt.background(rand_running_time, hp)
    return acc, apy, reward, stake_ratio, apr

def multi_trial_exp(kp, ki, kd, distribution = [], hp=True):
    global highest_apr
    global highest_acc
    global highest_staked
    global highest_gain
    global lowest_apr2target_diff
    new_record=False
    accs = []
    aprs = []
    rewards = []
    stakes_ratios = []
    aprs = []
    for i in range(0, AVG_LEN):
        acc, apy, reward, stake_ratio, apr = experiment(CONTROLLER_TYPE_DISCRETE, rkp=kp, rki=ki, rkd=kd, distribution=distribution, hp=hp)
        accs += [acc]
        rewards += [reward]
        aprs += [apr]
        stakes_ratios += [stake_ratio]
    avg_acc = float(sum(accs))/AVG_LEN
    avg_reward = float(sum(rewards))/AVG_LEN
    avg_staked = float(sum(stakes_ratios))/AVG_LEN
    avg_apr = float(sum(aprs))/AVG_LEN
    buff = 'avg(acc): {}, avg(apr): {}, avg(reward): {}, avg(stake ratio): {}, kp: {}, ki:{}, kd:{}'.format(avg_acc, avg_apr, avg_reward, avg_staked, kp, ki, kd)
    if avg_apr > 0:
        gain = (kp, ki, kd)
        acc_gain = (avg_apr, gain)
        apr2target_diff = math.fabs(avg_apr - float(TARGET_APR))
        if  avg_acc > highest_acc and apr2target_diff < 0.08:
            new_record = True
            highest_apr = avg_apr
            highest_acc = avg_acc
            highest_staked = avg_staked
            highest_gain = (kp, ki, kd)
            lowest_apr2target_diff = apr2target_diff
            with open('log'+os.sep+"highest_gain.txt", 'w') as f:
                f.write(buff)
    return buff, new_record

def crawler(crawl, range_multiplier, step=0.1):
    start = None
    if crawl==KP:
        start = highest_gain[0]
    elif crawl==KI:
        start = highest_gain[1]
    elif crawl==KD:
        start = highest_gain[2]

    range_start = (start*range_multiplier if start <=0 else -1*start)
    range_end = (-1*start if start<=0 else range_multiplier*start)
    # if number of steps under 10 step resize the step to 50
    while (range_end-range_start)/step < 10:
        range_start -= SHIFTING
        range_end += SHIFTING
        step /= 10

    while True:
        try:
            crawl_range = np.arange(range_start, range_end, step)
            break
        except Exception as e:
            print('start: {}, end: {}, step: {}, exp: {}'.format(range_start, rang_end, step, e))
            step*=10
    np.random.shuffle(crawl_range)
    crawl_range = tqdm(crawl_range)
    distribution = [random.gauss(ERC20DRK/NODES, ERC20DRK/NODES*0.1) for i in range(NODES)]
    for i in crawl_range:
        kp = i if crawl==KP else highest_gain[0]
        ki = i if crawl==KI else highest_gain[1]
        kd = i if crawl==KD else highest_gain[2]
        buff, new_record = multi_trial_exp(kp, ki, kd, distribution, hp=high_precision)
        crawl_range.set_description('highest:{} / {}'.format(highest_acc, buff))
        if new_record:
            break

while True:
    prev_highest_gain = highest_gain
    # kp crawl
    crawler(KP, KP_RANGE_MULTIPLIER, KP_STEP)
    if highest_gain[0] == prev_highest_gain[0]:
        KP_RANGE_MULTIPLIER+=1
        KP_STEP/=10
    else:
        start = highest_gain[0]
        range_start = (start*KP_RANGE_MULTIPLIER if start <=0 else -1*start) - SHIFTING
        range_end = (-1*start if start<=0 else KP_RANGE_MULTIPLIER*start) + SHIFTING
        while (range_end - range_start)/KP_STEP >500:
            #if KP_STEP < 0.1:
            KP_STEP*=2
            KP_RANGE_MULTIPLIER-=1
            #TODO (res) shouldn't the range also shrink?
            # not always true.
            # how to distinguish between thrinking range, and large step?
            # good strategy is step shoudn't > 0.1
            # range also should be > 0.8
            # what about range multiplier?

    # ki crawl
    crawler(KI, KI_RANGE_MULTIPLIER, KI_STEP)
    if highest_gain[1] == prev_highest_gain[1]:
        KI_RANGE_MULTIPLIER+=1
        KI_STEP/=10
    else:
        start = highest_gain[1]
        range_start = (start*KI_RANGE_MULTIPLIER if start <=0 else -1*start) - SHIFTING
        range_end = (-1*start if start<=0 else KI_RANGE_MULTIPLIER*start) + SHIFTING
        while (range_end - range_start)/KI_STEP >500:
            #print('range_end: {}, range_start: {}, ki_step: {}'.format(range_end, range_start, KI_STEP))
            #if KP_STEP < 1:
            KI_STEP*=2
            KI_RANGE_MULTIPLIER-=1
    # kd crawl
    crawler(KD, KD_RANGE_MULTIPLIER, KD_STEP)
    if highest_gain[2] == prev_highest_gain[2]:
        KD_RANGE_MULTIPLIER+=1
        KD_STEP/=10
    else:
        start = highest_gain[2]
        range_start = (start*KD_RANGE_MULTIPLIER if start <=0 else -1*start) - SHIFTING
        range_end = (-1*start if start<=0 else KD_RANGE_MULTIPLIER*start) + SHIFTING
        while (range_end - range_start)/KD_STEP >500:
            #if KD_STEP < 0.1:
            KD_STEP*=2
            KD_RANGE_MULTIPLIER-=1
