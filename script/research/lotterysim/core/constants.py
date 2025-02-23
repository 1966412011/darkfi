from decimal import Decimal as Num

N_TERM = 2
CONTROLLER_TYPE_ANALOGUE=-1
CONTROLLER_TYPE_DISCRETE=0
CONTROLLER_TYPE_TAKAHASHI=1
ERC20DRK=2.1*10**9

L = 28948022309329048855892746252171976963363056481941560715954676764349967630337.0

F_MIN = 0.0001
F_MAX = 0.9999

REWARD_MIN = 1
REWARD_MAX = 1000

SLOT = 90
ONE_YEAR = Num(365.25*24*60*60/SLOT)
ONE_MONTH = int(30*24*60*60/SLOT)
VESTING_PERIOD = ONE_MONTH

TARGET_APR = Num(0.12)

PRIMARY_REWARD_TARGET = 0.35 # staked ratio
SECONDARY_LEAD_TARGET = 1 #number of lead per slot

EPSILON = 1
EPOCH_LENGTH = Num(10)

L_HP = Num(L)
F_MIN_HP = Num(F_MIN)
F_MAX_HP = Num(F_MAX)
EPSILON_HP = Num(EPSILON)
REWARD_MIN_HP = Num(REWARD_MIN)
REWARD_MAX_HP = Num(REWARD_MAX)

ACC_WINDOW = 100
